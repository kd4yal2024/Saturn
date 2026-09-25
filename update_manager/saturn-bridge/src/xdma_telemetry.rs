//! Best-effort structured telemetry for isolated direct-XDMA validation probes.
//!
//! Probe safety and cleanup always take precedence over diagnostics. Snapshot
//! failures are logged but never change a probe result.

use std::env;
use std::fmt::Display;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_SNAPSHOT_PATH: &str = "/var/lib/saturn-state/xdma-telemetry.json";
const SNAPSHOT_PATH_ENV: &str = "SATURN_BRIDGE_XDMA_TELEMETRY_PATH";

static PROBE_OUTCOME_RECORDED: AtomicBool = AtomicBool::new(false);

/// Fixed storage and no allocation on the measured path. Durations round up
/// to microseconds; p99 is the upper bound of a power-of-two bucket, not an
/// exact percentile. The last bucket uses the observed maximum as its bound.
#[derive(Clone, Debug, Default)]
pub(crate) struct LatencyHistogram {
    buckets: [u64; 32],
    count: u64,
    max_us: u64,
}

impl LatencyHistogram {
    pub(crate) fn record(&mut self, duration: Duration) {
        let us = duration
            .as_nanos()
            .div_ceil(1_000)
            .min(u128::from(u64::MAX)) as u64;
        let bucket = (u64::BITS - us.saturating_sub(1).leading_zeros()).min(31) as usize;
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
        self.count = self.count.saturating_add(1);
        self.max_us = self.max_us.max(us);
    }

    fn p99_upper_us(&self) -> u64 {
        // ceil(count * 99 / 100), without overflowing the product.
        let rank = self.count - self.count / 100;
        let mut cumulative = 0u64;
        for (index, count) in self.buckets.iter().enumerate() {
            cumulative = cumulative.saturating_add(*count);
            if cumulative >= rank {
                return if index == 31 {
                    self.max_us
                } else {
                    (1u64 << index).min(self.max_us)
                };
            }
        }
        self.max_us
    }

    /// Formatting is only for the low-rate telemetry publisher, never the
    /// reader. Prefixes distinguish session totals from reporting intervals.
    pub(crate) fn fields(&self, prefix: &str) -> [(String, TelemetryValue); 3] {
        [
            (
                format!("{prefix}_count"),
                TelemetryValue::number(self.count),
            ),
            (
                format!("{prefix}_max_us"),
                TelemetryValue::number(self.max_us),
            ),
            (
                format!("{prefix}_p99_upper_us"),
                TelemetryValue::number(self.p99_upper_us()),
            ),
        ]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TelemetryValue {
    Bool(bool),
    Number(String),
    Text(String),
}

impl TelemetryValue {
    pub(crate) fn boolean(value: bool) -> Self {
        Self::Bool(value)
    }

    pub(crate) fn number(value: impl Display) -> Self {
        Self::Number(value.to_string())
    }

    pub(crate) fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    fn to_json(&self) -> String {
        match self {
            Self::Bool(value) => value.to_string(),
            Self::Number(value) => {
                if value.parse::<f64>().is_ok_and(|number| number.is_finite()) {
                    value.clone()
                } else {
                    "null".to_string()
                }
            }
            Self::Text(value) => json_string(value),
        }
    }
}

pub(crate) fn record_probe_outcome(
    phase: u8,
    probe: &str,
    status: &str,
    cleanup: &str,
    error: Option<&str>,
    metrics: &[(&str, TelemetryValue)],
) {
    PROBE_OUTCOME_RECORDED.store(true, Ordering::Release);
    let path = snapshot_path();
    let document = serialize_snapshot(phase, probe, status, cleanup, error, metrics);
    if let Err(write_error) = write_snapshot_atomic(&path, document.as_bytes(), true) {
        eprintln!(
            "saturn-bridge: could not persist XDMA telemetry snapshot {}: {}",
            path.display(),
            write_error
        );
    }
}

pub(crate) fn record_probe_failure_if_unrecorded(phase: u8, probe: &str, error: &str) {
    if PROBE_OUTCOME_RECORDED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let path = snapshot_path();
    let document = serialize_snapshot(phase, probe, "failed", "guarded", Some(error), &[]);
    if let Err(write_error) = write_snapshot_atomic(&path, document.as_bytes(), true) {
        eprintln!(
            "saturn-bridge: could not persist XDMA telemetry snapshot {}: {}",
            path.display(),
            write_error
        );
    }
}

pub(crate) fn record_runtime_readiness(
    path: &Path,
    status: &str,
    error: Option<&str>,
    metrics: &[(&str, TelemetryValue)],
) -> io::Result<()> {
    let updated_at_ms = unix_time_ms();
    let mut document = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"updated_at_ms\": {},\n",
            "  \"source\": \"saturn-bridge\",\n",
            "  \"backend\": \"xdma\",\n",
            "  \"status\": {},\n",
            "  \"rf_safe\": true,\n",
            "  \"error\": {},\n",
            "  \"metrics\": {{"
        ),
        updated_at_ms,
        json_string(status),
        error.map_or_else(|| "null".to_string(), json_string),
    );
    for (index, (name, value)) in metrics.iter().enumerate() {
        if index == 0 {
            document.push('\n');
        } else {
            document.push_str(",\n");
        }
        document.push_str("    ");
        document.push_str(&json_string(name));
        document.push_str(": ");
        document.push_str(&value.to_json());
    }
    if metrics.is_empty() {
        document.push_str("}\n}\n");
    } else {
        document.push_str("\n  }\n}\n");
    }
    // Runtime records live in /run and are regenerated every second. Atomic
    // rename provides reader coherence; forcing an fsync adds latency to the
    // RX owner without useful crash durability for ephemeral data.
    write_snapshot_atomic(path, document.as_bytes(), false)
}

pub(crate) fn record_runtime_performance(
    path: &Path,
    status: &str,
    metrics: &[(&str, TelemetryValue)],
) -> io::Result<()> {
    record_runtime_readiness(path, status, None, metrics)
}

fn snapshot_path() -> PathBuf {
    env::var_os(SNAPSHOT_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SNAPSHOT_PATH))
}

fn serialize_snapshot(
    phase: u8,
    probe: &str,
    status: &str,
    cleanup: &str,
    error: Option<&str>,
    metrics: &[(&str, TelemetryValue)],
) -> String {
    let updated_at_ms = unix_time_ms();
    let mut document = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"updated_at_ms\": {},\n",
            "  \"source\": \"saturn-bridge\",\n",
            "  \"phase\": {},\n",
            "  \"probe\": {},\n",
            "  \"status\": {},\n",
            "  \"cleanup\": {},\n",
            "  \"error\": {},\n",
            "  \"metrics\": {{"
        ),
        updated_at_ms,
        phase,
        json_string(probe),
        json_string(status),
        json_string(cleanup),
        error.map_or_else(|| "null".to_string(), json_string),
    );
    for (index, (name, value)) in metrics.iter().enumerate() {
        if index == 0 {
            document.push('\n');
        } else {
            document.push_str(",\n");
        }
        document.push_str("    ");
        document.push_str(&json_string(name));
        document.push_str(": ");
        document.push_str(&value.to_json());
    }
    if metrics.is_empty() {
        document.push_str("}\n}\n");
    } else {
        document.push_str("\n  }\n}\n");
    }
    document
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character <= '\u{001f}' => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

fn write_snapshot_atomic(path: &Path, contents: &[u8], durable: bool) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "XDMA telemetry path has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let temporary = parent.join(format!(
        ".xdma-telemetry-{}-{nonce}.tmp",
        std::process::id()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.set_permissions(fs::Permissions::from_mode(0o644))?;
        file.write_all(contents)?;
        if durable {
            file.sync_all()?;
        }
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{serialize_snapshot, write_snapshot_atomic, LatencyHistogram, TelemetryValue};
    use std::fs;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn latency_histogram_reports_bounded_p99_and_exact_rounded_max() {
        let mut latency = LatencyHistogram::default();
        assert_eq!(latency.p99_upper_us(), 0);
        latency.record(Duration::ZERO);
        latency.record(Duration::from_nanos(1));
        latency.record(Duration::from_nanos(1001));
        assert_eq!(latency.buckets[0], 2);
        assert_eq!(latency.buckets[1], 1);
        assert_eq!(latency.max_us, 2);
        assert_eq!(latency.p99_upper_us(), 2);

        let mut latency = LatencyHistogram::default();
        for _ in 0..99 {
            latency.record(Duration::from_micros(9));
        }
        latency.record(Duration::from_micros(1000));
        assert_eq!(latency.count, 100);
        assert_eq!(latency.p99_upper_us(), 16);
        assert_eq!(latency.max_us, 1000);
        assert_eq!(
            latency.fields("test")[2],
            ("test_p99_upper_us".into(), TelemetryValue::number(16))
        );
        let mut long = LatencyHistogram::default();
        long.record(Duration::MAX);
        assert_eq!(long.p99_upper_us(), u64::MAX);
    }

    #[test]
    fn snapshot_json_escapes_errors_and_preserves_metric_types() {
        let document = serialize_snapshot(
            5,
            "guarded-tx",
            "failed",
            "verified",
            Some("bad \"power\"\nreading"),
            &[
                ("fifo_lwm", TelemetryValue::number(2342)),
                ("rf_cleanup", TelemetryValue::boolean(true)),
                ("antenna", TelemetryValue::text("ANT1")),
            ],
        );
        assert!(document.contains("\"phase\": 5"));
        assert!(document.contains("\"error\": \"bad \\\"power\\\"\\nreading\""));
        assert!(document.contains("\"fifo_lwm\": 2342"));
        assert!(document.contains("\"rf_cleanup\": true"));
        assert!(document.contains("\"antenna\": \"ANT1\""));
    }

    #[test]
    fn snapshot_write_atomically_replaces_the_previous_file() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "saturn-xdma-telemetry-test-{}-{nonce}",
            std::process::id()
        ));
        let path = root.join("xdma-telemetry.json");
        write_snapshot_atomic(&path, b"{\"status\":\"first\"}\n", true).unwrap();
        write_snapshot_atomic(&path, b"{\"status\":\"second\"}\n", true).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\"status\":\"second\"}\n"
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
