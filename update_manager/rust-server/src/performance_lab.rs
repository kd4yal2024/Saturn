use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

use crate::state_store::{write_json_atomic, AtomicWriteOptions};

pub const MAX_BENCHMARK_RUNS: usize = 64;
const MAX_METRICS: usize = 64;
const MAX_EVENTS: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkMetricSummary {
    pub count: u32,
    pub min: f64,
    pub mean: f64,
    pub p95: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceBenchmarkRun {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub operator_observation: String,
    pub captured_at_ms: u64,
    pub captured_at_iso: String,
    pub duration_seconds: f64,
    pub sample_interval_seconds: f64,
    pub sample_count: u32,
    pub workload_key: String,
    pub selected_app: String,
    #[serde(default)]
    pub startup_mode: String,
    #[serde(default)]
    pub panel_mode: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub build_commit: String,
    #[serde(default)]
    pub artifact_sha256: String,
    pub metrics: BTreeMap<String, BenchmarkMetricSummary>,
    #[serde(default)]
    pub events: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceBenchmarkFile {
    pub schema_version: u32,
    #[serde(default)]
    pub runs: Vec<PerformanceBenchmarkRun>,
}

impl Default for PerformanceBenchmarkFile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            runs: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BenchmarkDeleteRequest {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct BenchmarkCompareRequest {
    pub baseline_id: String,
    pub candidate_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BenchmarkCheck {
    pub key: String,
    pub label: String,
    pub status: String,
    pub baseline: Option<f64>,
    pub candidate: Option<f64>,
    pub delta_percent: Option<f64>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BenchmarkComparison {
    pub verdict: String,
    pub compatible: bool,
    pub baseline_id: String,
    pub candidate_id: String,
    pub summary: String,
    pub checks: Vec<BenchmarkCheck>,
}

pub async fn load_benchmark_file(path: &Path) -> Result<PerformanceBenchmarkFile, String> {
    let raw = match tokio::fs::read(path).await {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PerformanceBenchmarkFile::default())
        }
        Err(error) => {
            return Err(format!(
                "failed to read performance benchmark history {}: {error}",
                path.display()
            ))
        }
    };
    let file: PerformanceBenchmarkFile = serde_json::from_slice(&raw).map_err(|error| {
        format!(
            "failed to parse performance benchmark history {}: {error}",
            path.display()
        )
    })?;
    if file.schema_version != 1 {
        return Err(format!(
            "unsupported performance benchmark schema version {}",
            file.schema_version
        ));
    }
    for run in &file.runs {
        validate_run(run)?;
    }
    Ok(file)
}

pub async fn save_run(
    path: &Path,
    run: PerformanceBenchmarkRun,
) -> Result<PerformanceBenchmarkFile, String> {
    validate_run(&run)?;
    let mut file = load_benchmark_file(path).await?;
    file.runs.retain(|existing| existing.id != run.id);
    file.runs.push(run);
    file.runs
        .sort_by(|a, b| b.captured_at_ms.cmp(&a.captured_at_ms));
    file.runs.truncate(MAX_BENCHMARK_RUNS);
    write_json_atomic(path, &file, AtomicWriteOptions::state_file()).await?;
    Ok(file)
}

pub async fn delete_run(path: &Path, id: &str) -> Result<PerformanceBenchmarkFile, String> {
    validate_identifier(id)?;
    let mut file = load_benchmark_file(path).await?;
    let before = file.runs.len();
    file.runs.retain(|run| run.id != id);
    if file.runs.len() == before {
        return Err(format!("benchmark run not found: {id}"));
    }
    write_json_atomic(path, &file, AtomicWriteOptions::state_file()).await?;
    Ok(file)
}

pub fn compare_by_id(
    file: &PerformanceBenchmarkFile,
    baseline_id: &str,
    candidate_id: &str,
) -> Result<BenchmarkComparison, String> {
    validate_identifier(baseline_id)?;
    validate_identifier(candidate_id)?;
    let baseline = file
        .runs
        .iter()
        .find(|run| run.id == baseline_id)
        .ok_or_else(|| format!("baseline benchmark not found: {baseline_id}"))?;
    let candidate = file
        .runs
        .iter()
        .find(|run| run.id == candidate_id)
        .ok_or_else(|| format!("candidate benchmark not found: {candidate_id}"))?;
    Ok(compare_runs(baseline, candidate))
}

pub fn validate_run(run: &PerformanceBenchmarkRun) -> Result<(), String> {
    if run.schema_version != 1 {
        return Err("benchmark schema_version must be 1".to_string());
    }
    validate_identifier(&run.id)?;
    validate_text("name", &run.name, 1, 96)?;
    validate_text("notes", &run.notes, 0, 800)?;
    validate_text("operator_observation", &run.operator_observation, 0, 800)?;
    validate_text("captured_at_iso", &run.captured_at_iso, 1, 64)?;
    validate_text("workload_key", &run.workload_key, 1, 512)?;
    validate_text("selected_app", &run.selected_app, 1, 32)?;
    validate_text("startup_mode", &run.startup_mode, 0, 32)?;
    validate_text("panel_mode", &run.panel_mode, 0, 32)?;
    validate_text("backend", &run.backend, 0, 32)?;
    validate_text("build_commit", &run.build_commit, 0, 64)?;
    validate_text("artifact_sha256", &run.artifact_sha256, 0, 64)?;
    if !run.artifact_sha256.is_empty()
        && (run.artifact_sha256.len() != 64
            || !run
                .artifact_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(
            "artifact_sha256 must be an empty or 64-character hexadecimal digest".to_string(),
        );
    }
    if !(10.0..=1800.0).contains(&run.duration_seconds) {
        return Err("duration_seconds must be between 10 and 1800".to_string());
    }
    if !(0.25..=30.0).contains(&run.sample_interval_seconds) {
        return Err("sample_interval_seconds must be between 0.25 and 30".to_string());
    }
    if !(5..=3600).contains(&run.sample_count) {
        return Err("sample_count must be between 5 and 3600".to_string());
    }
    if run.metrics.is_empty() || run.metrics.len() > MAX_METRICS {
        return Err(format!(
            "metrics must contain between 1 and {MAX_METRICS} entries"
        ));
    }
    if run.events.len() > MAX_EVENTS {
        return Err(format!("events may contain at most {MAX_EVENTS} entries"));
    }
    for (key, metric) in &run.metrics {
        validate_metric_key(key)?;
        if metric.count == 0 || metric.count > run.sample_count {
            return Err(format!("metric {key} has an invalid count"));
        }
        if ![metric.min, metric.mean, metric.p95, metric.max]
            .iter()
            .all(|value| value.is_finite())
        {
            return Err(format!("metric {key} contains a non-finite value"));
        }
        if metric.min > metric.mean || metric.mean > metric.max {
            return Err(format!("metric {key} has inconsistent min/mean/max values"));
        }
        if metric.p95 < metric.min || metric.p95 > metric.max {
            return Err(format!("metric {key} has an inconsistent p95 value"));
        }
    }
    for (key, value) in &run.events {
        validate_metric_key(key)?;
        if !value.is_finite() || *value < 0.0 {
            return Err(format!("event {key} must be finite and non-negative"));
        }
    }
    Ok(())
}

fn validate_identifier(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 80
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(
            "benchmark id must use 1-80 ASCII letters, digits, '.', '_' or '-'".to_string(),
        );
    }
    Ok(())
}

fn validate_metric_key(key: &str) -> Result<(), String> {
    if key.is_empty()
        || key.len() > 64
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(format!("invalid benchmark metric key: {key}"));
    }
    Ok(())
}

fn validate_text(label: &str, value: &str, min: usize, max: usize) -> Result<(), String> {
    let len = value.chars().count();
    if len < min || len > max || value.chars().any(char::is_control) {
        return Err(format!(
            "{label} must contain {min}-{max} printable characters"
        ));
    }
    Ok(())
}

fn metric_mean(run: &PerformanceBenchmarkRun, key: &str) -> Option<f64> {
    run.metrics.get(key).map(|metric| metric.mean)
}

fn metric_p95(run: &PerformanceBenchmarkRun, key: &str) -> Option<f64> {
    run.metrics.get(key).map(|metric| metric.p95)
}

fn delta_percent(baseline: f64, candidate: f64) -> Option<f64> {
    if baseline.abs() < 1e-9 {
        None
    } else {
        Some(((candidate - baseline) / baseline) * 100.0)
    }
}

fn push_check(
    checks: &mut Vec<BenchmarkCheck>,
    key: &str,
    label: &str,
    status: &str,
    baseline: Option<f64>,
    candidate: Option<f64>,
    message: String,
) {
    checks.push(BenchmarkCheck {
        key: key.to_string(),
        label: label.to_string(),
        status: status.to_string(),
        baseline,
        candidate,
        delta_percent: baseline
            .zip(candidate)
            .and_then(|(base, next)| delta_percent(base, next)),
        message,
    });
}

fn compare_lower_is_better(
    checks: &mut Vec<BenchmarkCheck>,
    baseline: &PerformanceBenchmarkRun,
    candidate: &PerformanceBenchmarkRun,
    key: &str,
    label: &str,
    review_fraction: f64,
    reject_fraction: f64,
    absolute_review: f64,
    absolute_reject: f64,
) {
    let (Some(base), Some(next)) = (metric_mean(baseline, key), metric_mean(candidate, key)) else {
        push_check(
            checks,
            key,
            label,
            "review",
            metric_mean(baseline, key),
            metric_mean(candidate, key),
            "metric is missing from one run".to_string(),
        );
        return;
    };
    let increase = next - base;
    let reject_limit = (base.abs() * reject_fraction).max(absolute_reject);
    let review_limit = (base.abs() * review_fraction).max(absolute_review);
    let (status, message) = if increase > reject_limit {
        (
            "reject",
            format!("regressed by {increase:.2}; allowed increase is {reject_limit:.2}"),
        )
    } else if increase > review_limit {
        (
            "review",
            format!("increased by {increase:.2}; inspect the tradeoff"),
        )
    } else {
        ("pass", format!("change {increase:+.2} is within tolerance"))
    };
    push_check(checks, key, label, status, Some(base), Some(next), message);
}

fn compare_minimum_throughput(
    checks: &mut Vec<BenchmarkCheck>,
    baseline: &PerformanceBenchmarkRun,
    candidate: &PerformanceBenchmarkRun,
    key: &str,
    label: &str,
) {
    let (Some(base), Some(next)) = (metric_mean(baseline, key), metric_mean(candidate, key)) else {
        push_check(
            checks,
            key,
            label,
            "review",
            metric_mean(baseline, key),
            metric_mean(candidate, key),
            "metric is missing from one run".to_string(),
        );
        return;
    };
    let ratio = if base.abs() < 1e-9 { 1.0 } else { next / base };
    let (status, message) = if ratio < 0.97 {
        (
            "reject",
            format!("throughput fell to {:.1}% of baseline", ratio * 100.0),
        )
    } else if ratio < 0.985 {
        (
            "review",
            format!("throughput fell to {:.1}% of baseline", ratio * 100.0),
        )
    } else {
        (
            "pass",
            format!("throughput is {:.1}% of baseline", ratio * 100.0),
        )
    };
    push_check(checks, key, label, status, Some(base), Some(next), message);
}

pub fn compare_runs(
    baseline: &PerformanceBenchmarkRun,
    candidate: &PerformanceBenchmarkRun,
) -> BenchmarkComparison {
    let mut checks = Vec::new();
    let compatible = !baseline.workload_key.is_empty()
        && baseline.workload_key == candidate.workload_key
        && baseline.selected_app == candidate.selected_app
        && baseline.backend == candidate.backend;
    if !compatible {
        push_check(
            &mut checks,
            "workload_identity",
            "Workload identity",
            "incompatible",
            None,
            None,
            "backend, application, or workload configuration differs".to_string(),
        );
        return BenchmarkComparison {
            verdict: "incompatible".to_string(),
            compatible: false,
            baseline_id: baseline.id.clone(),
            candidate_id: candidate.id.clone(),
            summary: "Runs are not directly comparable because their workload identity differs."
                .to_string(),
            checks,
        };
    }

    compare_lower_is_better(
        &mut checks,
        baseline,
        candidate,
        "cpu_pct_core",
        "CPU per core",
        0.05,
        0.10,
        1.5,
        3.0,
    );
    compare_lower_is_better(
        &mut checks,
        baseline,
        candidate,
        "scheduler_delay_ms_per_sec",
        "Scheduler delay",
        0.10,
        0.20,
        1.0,
        2.0,
    );
    compare_lower_is_better(
        &mut checks,
        baseline,
        candidate,
        "xdma_irq_per_mib",
        "XDMA IRQ efficiency",
        0.08,
        0.15,
        0.5,
        1.0,
    );
    compare_lower_is_better(
        &mut checks,
        baseline,
        candidate,
        "rss_mb",
        "Resident memory",
        0.08,
        0.15,
        5.0,
        10.0,
    );
    compare_minimum_throughput(
        &mut checks,
        baseline,
        candidate,
        "ddc_packets_per_sec",
        "DDC packet throughput",
    );

    if let (Some(base), Some(next)) = (
        metric_p95(baseline, "scheduler_delay_ms_per_sec"),
        metric_p95(candidate, "scheduler_delay_ms_per_sec"),
    ) {
        let increase = next - base;
        let status = if increase > (base.abs() * 0.25).max(5.0) {
            "reject"
        } else if increase > (base.abs() * 0.12).max(2.0) {
            "review"
        } else {
            "pass"
        };
        push_check(
            &mut checks,
            "scheduler_delay_p95",
            "Scheduler delay p95",
            status,
            Some(base),
            Some(next),
            format!("p95 change {increase:+.2} ms/s"),
        );
    }

    for (key, label) in [
        ("app_errors", "Application errors"),
        ("fifo_faults", "FIFO faults"),
        ("network_errors", "Network errors/drops"),
        ("adc_overflows", "ADC overflow events"),
    ] {
        let base = baseline.events.get(key).copied().unwrap_or(0.0);
        let next = candidate.events.get(key).copied().unwrap_or(0.0);
        let status = if next > 0.0 { "reject" } else { "pass" };
        let message = if next > 0.0 {
            format!("candidate recorded {next:.0} event(s)")
        } else {
            "candidate recorded zero events".to_string()
        };
        push_check(
            &mut checks,
            key,
            label,
            status,
            Some(base),
            Some(next),
            message,
        );
    }

    let verdict = if checks.iter().any(|check| check.status == "reject") {
        "reject"
    } else if checks.iter().any(|check| check.status == "review") {
        "review"
    } else {
        "accept"
    };
    let summary = match verdict {
        "accept" => "Candidate met the current no-regression gates for this workload.",
        "review" => {
            "Candidate has no hard failure, but one or more changes require engineering review."
        }
        _ => "Candidate failed one or more no-regression gates.",
    };
    BenchmarkComparison {
        verdict: verdict.to_string(),
        compatible: true,
        baseline_id: baseline.id.clone(),
        candidate_id: candidate.id.clone(),
        summary: summary.to_string(),
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(mean: f64) -> BenchmarkMetricSummary {
        BenchmarkMetricSummary {
            count: 30,
            min: mean * 0.9,
            mean,
            p95: mean * 1.05,
            max: mean * 1.1,
        }
    }

    fn run(id: &str) -> PerformanceBenchmarkRun {
        PerformanceBenchmarkRun {
            schema_version: 1,
            id: id.to_string(),
            name: id.to_string(),
            notes: String::new(),
            operator_observation: "Sounded clean".to_string(),
            captured_at_ms: 1,
            captured_at_iso: "2026-08-28T12:00:00Z".to_string(),
            duration_seconds: 60.0,
            sample_interval_seconds: 2.0,
            sample_count: 30,
            workload_key: "p2|panel|1rx|192k".to_string(),
            selected_app: "p2".to_string(),
            startup_mode: "panel".to_string(),
            panel_mode: "auto".to_string(),
            backend: "p2".to_string(),
            build_commit: "0123456789012345678901234567890123456789".to_string(),
            artifact_sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
            metrics: BTreeMap::from([
                ("cpu_pct_core".to_string(), metric(20.0)),
                ("rss_mb".to_string(), metric(100.0)),
                ("scheduler_delay_ms_per_sec".to_string(), metric(2.0)),
                ("xdma_irq_per_mib".to_string(), metric(10.0)),
                ("ddc_packets_per_sec".to_string(), metric(800.0)),
            ]),
            events: BTreeMap::from([
                ("app_errors".to_string(), 0.0),
                ("fifo_faults".to_string(), 0.0),
                ("network_errors".to_string(), 0.0),
                ("adc_overflows".to_string(), 0.0),
            ]),
        }
    }

    #[test]
    fn validates_a_bounded_run() {
        assert!(validate_run(&run("baseline")).is_ok());
        let mut invalid = run("bad id");
        assert!(validate_run(&invalid).is_err());
        invalid = run("valid-id");
        invalid.metrics.get_mut("cpu_pct_core").unwrap().mean = f64::NAN;
        assert!(validate_run(&invalid).is_err());
    }

    #[test]
    fn compatible_clean_candidate_is_accepted() {
        let baseline = run("baseline");
        let mut candidate = run("candidate");
        candidate.metrics.get_mut("cpu_pct_core").unwrap().mean = 19.0;
        assert_eq!(compare_runs(&baseline, &candidate).verdict, "accept");
    }

    #[test]
    fn regression_and_fault_event_are_rejected() {
        let baseline = run("baseline");
        let mut candidate = run("candidate");
        candidate.metrics.get_mut("cpu_pct_core").unwrap().mean = 25.0;
        candidate.events.insert("fifo_faults".to_string(), 1.0);
        let comparison = compare_runs(&baseline, &candidate);
        assert_eq!(comparison.verdict, "reject");
        assert!(comparison
            .checks
            .iter()
            .any(|check| check.key == "fifo_faults" && check.status == "reject"));
    }

    #[test]
    fn different_workloads_are_incompatible() {
        let baseline = run("baseline");
        let mut candidate = run("candidate");
        candidate.workload_key = "different".to_string();
        let comparison = compare_runs(&baseline, &candidate);
        assert_eq!(comparison.verdict, "incompatible");
        assert!(!comparison.compatible);
    }
}
