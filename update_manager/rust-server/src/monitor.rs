use axum::{
    extract::Query,
    response::{IntoResponse, Json},
};
use regex::RegexBuilder;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{Disks, Networks, System};
use tokio::process::Command;
use tracing::error;

use crate::sync_ext::MutexExt;

#[derive(Deserialize, Default)]
pub struct ProcQuery {
    proc_sort: Option<String>,
    proc_order: Option<String>,
    proc_user: Option<String>,
    proc_regex: Option<String>,
    proc_top: Option<usize>,
    proc_page: Option<usize>,
    proc_page_size: Option<usize>,
    rate_scope: Option<String>,
}

pub async fn get_system_data(Query(q): Query<ProcQuery>) -> impl IntoResponse {
    // Keep overview polling from advancing the Monitor page's rate baseline.
    // Only fixed scopes are accepted so callers cannot grow the global rate map.
    let rate_scope = match q.rate_scope.as_deref() {
        Some("overview") => "overview",
        _ => "monitor",
    };
    let cpu = match read_per_core_cpu(rate_scope).await {
        Ok(v) => v,
        Err(e) => {
            error!("cpu read error: {e}");
            vec![0.0]
        }
    };

    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu();
    sys.refresh_processes();

    let total_mem_kb = sys.total_memory() as f64;
    let avail_mem_kb = sys.available_memory() as f64;
    let used_mem_kb = (total_mem_kb - avail_mem_kb).max(0.0);

    let m_total_gb = total_mem_kb / 1024.0 / 1024.0;
    let m_used_gb = used_mem_kb / 1024.0 / 1024.0;
    let m_percent = if total_mem_kb > 0.0 {
        (used_mem_kb / total_mem_kb) * 100.0
    } else {
        0.0
    };

    let mut disks = Disks::new_with_refreshed_list();
    disks.refresh();
    let (d_total_gb, d_used_gb, d_percent) = pick_root_disk(&disks);
    let (d_read_bytes, d_write_bytes) = read_disk_io_totals();
    let (d_read_bps, d_write_bps) =
        calc_rate(&format!("disk:{rate_scope}"), d_read_bytes, d_write_bytes);

    let mut networks = Networks::new_with_refreshed_list();
    networks.refresh();
    let (mut sent, mut recv) = sum_networks(&networks);
    if sent == 0 && recv == 0 {
        let (psent, precv) = read_net_dev_totals();
        if psent > 0 || precv > 0 {
            sent = psent;
            recv = precv;
        }
    }
    let (tx_bps, rx_bps) = calc_rate(&format!("net:{rate_scope}"), sent, recv);

    let procs = list_procs_sysinfo(&sys, total_mem_kb, &q);
    let load = sysinfo::System::load_average();
    let uptime = sysinfo::System::uptime();
    let cpu_temp = read_cpu_temp_c();
    let swap_total_gb = sys.total_swap() as f64 / 1024.0 / 1024.0;
    let swap_used_gb = sys.used_swap() as f64 / 1024.0 / 1024.0;
    let swap_percent = if sys.total_swap() > 0 {
        (sys.used_swap() as f64 / sys.total_swap() as f64) * 100.0
    } else {
        0.0
    };

    Json(serde_json::json!({
        "cpu": cpu,
        "memory": { "percent": m_percent, "used": m_used_gb, "total": m_total_gb },
        "swap": { "percent": swap_percent, "used": swap_used_gb, "total": swap_total_gb },
        "disk": { "percent": d_percent, "used": d_used_gb, "total": d_total_gb, "read_bytes": d_read_bytes, "write_bytes": d_write_bytes, "read_bps": d_read_bps, "write_bps": d_write_bps },
        "network": { "sent": sent, "recv": recv, "tx_bps": tx_bps, "rx_bps": rx_bps },
        "load": { "one": load.one, "five": load.five, "fifteen": load.fifteen },
        "uptime": { "seconds": uptime },
        "temperature": { "cpu_c": cpu_temp },
        "processes": procs
    }))
}

pub async fn network_test() -> impl IntoResponse {
    let (sent0, recv0) = get_net_totals();
    let start = Instant::now();

    let urls = [
        "https://ash-speed.hetzner.com/10MB.bin",
        "https://proof.ovh.net/files/10Mb.dat",
        "https://speed.cloudflare.com/__down?bytes=10000000",
    ];
    let mut last_err = String::new();
    let mut ok = false;

    for url in urls {
        let out = Command::new("curl")
            .arg("-L")
            .arg("--silent")
            .arg("--show-error")
            .arg("--fail")
            .arg("--max-redirs")
            .arg("5")
            .arg("--output")
            .arg("/dev/null")
            .arg("--connect-timeout")
            .arg("5")
            .arg("--max-time")
            .arg("30")
            .arg(url)
            .output()
            .await;

        match out {
            Ok(o) if o.status.success() => {
                ok = true;
                break;
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
                if stderr.is_empty() {
                    last_err = format!("{} on {}", o.status, url);
                } else {
                    last_err = format!("{} on {} ({})", o.status, url, stderr);
                }
            }
            Err(e) => {
                last_err = format!("{} on {}", e, url);
            }
        }
    }

    let elapsed = start.elapsed().as_secs_f64().max(0.001);
    let (sent1, recv1) = get_net_totals();

    if ok {
        let tx_bps = ((sent1.saturating_sub(sent0)) as f64 / elapsed) as u64;
        let rx_bps = ((recv1.saturating_sub(recv0)) as f64 / elapsed) as u64;
        Json(serde_json::json!({
            "tx_bps": tx_bps,
            "rx_bps": rx_bps,
            "seconds": elapsed
        }))
    } else {
        Json(serde_json::json!({
            "error": format!("curl test failed: {}", if last_err.is_empty() { "no route succeeded".to_string() } else { last_err })
        }))
    }
}

struct CpuBaseline {
    snaps: Vec<CpuSnap>,
    at_ms: u128,
    last_result: Vec<f64>,
}

/// Below this interval, polls reuse the previous result instead of dividing
/// near-zero counter deltas.
const CPU_RATE_MIN_INTERVAL_MS: u128 = 250;

fn per_core_busy_percent(a: &[CpuSnap], b: &[CpuSnap]) -> Vec<f64> {
    a.iter()
        .zip(b.iter())
        .map(|(sa, sb)| {
            let d_idle = sb.idle.saturating_sub(sa.idle) as f64;
            let d_total = sb.total.saturating_sub(sa.total) as f64;
            if d_total > 0.0 {
                ((1.0 - d_idle / d_total) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            }
        })
        .collect()
}

// Like calc_rate, busy% is measured against the previous poll of the same
// scope, so the window is the page's whole refresh period instead of a 120ms
// in-request snapshot — which on this bursty radio workload was a dice roll,
// and measured the server's own request handling. Only the whitelisted
// rate_scope values reach this, so the map cannot grow unbounded.
async fn read_per_core_cpu(rate_scope: &str) -> Result<Vec<f64>, String> {
    static BASELINE: OnceLock<Mutex<HashMap<String, CpuBaseline>>> = OnceLock::new();
    let map = BASELINE.get_or_init(|| Mutex::new(HashMap::new()));

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let current = read_proc_stat().await?;
    if current.is_empty() {
        return Ok(vec![0.0]);
    }

    // Scope: the guard must not be held across an await point.
    {
        let mut guard = map.lock_unpoisoned();
        if let Some(base) = guard.get_mut(rate_scope) {
            if base.snaps.len() == current.len() {
                if now_ms.saturating_sub(base.at_ms) < CPU_RATE_MIN_INTERVAL_MS {
                    return Ok(base.last_result.clone());
                }
                let result = per_core_busy_percent(&base.snaps, &current);
                *base = CpuBaseline {
                    snaps: current,
                    at_ms: now_ms,
                    last_result: result.clone(),
                };
                return Ok(result);
            }
        }
    }

    // First poll for this scope (or a core-count change): fall back to one
    // short in-request window to have something to report.
    tokio::time::sleep(Duration::from_millis(120)).await;
    let second = read_proc_stat().await?;
    if second.len() != current.len() {
        return Ok(vec![0.0]);
    }
    let result = per_core_busy_percent(&current, &second);
    let at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    map.lock_unpoisoned().insert(
        rate_scope.to_string(),
        CpuBaseline {
            snaps: second,
            at_ms,
            last_result: result.clone(),
        },
    );
    Ok(result)
}

#[derive(Clone, Copy)]
struct CpuSnap {
    idle: u64,
    total: u64,
}

async fn read_proc_stat() -> Result<Vec<CpuSnap>, String> {
    let data = tokio::fs::read_to_string("/proc/stat")
        .await
        .map_err(|e| e.to_string())?;
    let mut res = Vec::new();
    for line in data.lines() {
        if !line.starts_with("cpu") || line.starts_with("cpu ") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 {
            continue;
        }
        let mut vals = Vec::new();
        for p in &parts[1..] {
            if let Ok(v) = p.parse::<u64>() {
                vals.push(v);
            }
        }
        if vals.len() < 5 {
            continue;
        }
        let idle = vals[3] + vals[4];
        let total: u64 = vals.iter().sum();
        res.push(CpuSnap { idle, total });
    }
    Ok(res)
}

fn pick_root_disk(disks: &Disks) -> (f64, f64, f64) {
    let mut total = 0.0;
    let mut used = 0.0;

    if let Some(d) = disks.iter().find(|d| d.mount_point() == Path::new("/")) {
        total = d.total_space() as f64;
        used = (d.total_space() - d.available_space()) as f64;
    } else if let Some(d) = disks.iter().next() {
        total = d.total_space() as f64;
        used = (d.total_space() - d.available_space()) as f64;
    }

    let total_gb = total / 1024.0 / 1024.0 / 1024.0;
    let used_gb = used / 1024.0 / 1024.0 / 1024.0;
    let percent = if total > 0.0 {
        (used / total) * 100.0
    } else {
        0.0
    };
    (total_gb, used_gb, percent)
}

fn sum_networks(networks: &Networks) -> (u64, u64) {
    let mut sent = 0u64;
    let mut recv = 0u64;
    for (_name, data) in networks.iter() {
        sent += data.transmitted();
        recv += data.received();
    }
    (sent, recv)
}

fn get_net_totals() -> (u64, u64) {
    let mut networks = Networks::new_with_refreshed_list();
    networks.refresh();
    let (sent, recv) = sum_networks(&networks);
    if sent == 0 && recv == 0 {
        return read_net_dev_totals();
    }
    (sent, recv)
}

fn read_net_dev_totals() -> (u64, u64) {
    if let Ok(data) = fs::read_to_string("/proc/net/dev") {
        let mut sent = 0u64;
        let mut recv = 0u64;
        for line in data.lines().skip(2) {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() != 2 {
                continue;
            }
            let stats: Vec<&str> = parts[1].split_whitespace().collect();
            if stats.len() >= 16 {
                recv += stats[0].parse::<u64>().unwrap_or(0);
                sent += stats[8].parse::<u64>().unwrap_or(0);
            }
        }
        return (sent, recv);
    }
    (0, 0)
}

fn read_cpu_temp_c() -> Option<f64> {
    if let Ok(entries) = fs::read_dir("/sys/class/thermal") {
        for entry in entries.flatten() {
            let path = entry.path().join("temp");
            if let Ok(s) = fs::read_to_string(&path) {
                if let Ok(raw) = s.trim().parse::<f64>() {
                    let c = if raw > 1000.0 { raw / 1000.0 } else { raw };
                    if c > 0.0 {
                        return Some(c);
                    }
                }
            }
        }
    }
    None
}

fn read_disk_io_totals() -> (u64, u64) {
    let dev = match root_device_name() {
        Some(d) => d,
        None => return (0, 0),
    };
    if let Ok(data) = fs::read_to_string("/proc/diskstats") {
        for line in data.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 14 {
                continue;
            }
            if parts[2] == dev {
                let sr = parts[5].parse::<u64>().unwrap_or(0);
                let sw = parts[9].parse::<u64>().unwrap_or(0);
                return (sr.saturating_mul(512), sw.saturating_mul(512));
            }
        }
    }
    (0, 0)
}

fn root_device_name() -> Option<String> {
    if let Ok(data) = fs::read_to_string("/proc/mounts") {
        for line in data.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == "/" {
                let dev = parts[0];
                if dev.starts_with("/dev/") {
                    return Some(base_device_name(dev));
                }
            }
        }
    }
    None
}

fn base_device_name(dev: &str) -> String {
    let name = dev.trim_start_matches("/dev/");
    if name.starts_with("nvme") && name.contains('p') {
        return name.split('p').next().unwrap_or(name).to_string();
    }
    if name.starts_with("mmcblk") && name.contains('p') {
        return name.split('p').next().unwrap_or(name).to_string();
    }
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit());
    if trimmed.is_empty() {
        name.to_string()
    } else {
        trimmed.to_string()
    }
}

fn calc_rate(kind: &str, a: u64, b: u64) -> (u64, u64) {
    static LAST: OnceLock<Mutex<std::collections::HashMap<String, (u64, u64, u128)>>> =
        OnceLock::new();
    let map = LAST.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut guard = map.lock_unpoisoned();

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let entry = guard.entry(kind.to_string()).or_insert((a, b, now_ms));
    let (la, lb, lt) = *entry;
    let dt_ms = (now_ms.saturating_sub(lt)).max(1);

    let ra = a.saturating_sub(la) * 1000 / dt_ms as u64;
    let rb = b.saturating_sub(lb) * 1000 / dt_ms as u64;

    *entry = (a, b, now_ms);
    (ra, rb)
}

fn read_passwd_user_map() -> HashMap<u32, String> {
    let Ok(contents) = fs::read_to_string("/etc/passwd") else {
        return HashMap::new();
    };

    contents
        .lines()
        .filter_map(|line| {
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let mut fields = line.split(':');
            let name = fields.next()?;
            fields.next()?;
            let uid = fields.next()?.parse::<u32>().ok()?;
            Some((uid, name.to_string()))
        })
        .collect()
}

#[derive(Debug)]
struct ProcInfo {
    pid: i32,
    user: String,
    cpu: f64,
    mem_pct: f64,
    mem_mb: f64,
    command: String,
    cwd: String,
    start_time: u64,
}

fn list_procs_sysinfo(sys: &System, total_mem_kb: f64, q: &ProcQuery) -> Vec<serde_json::Value> {
    let mut out: Vec<ProcInfo> = Vec::new();
    let passwd_users = read_passwd_user_map();
    for (pid, proc_) in sys.processes() {
        let pid_i32 = pid.as_u32() as i32;
        let user = proc_
            .user_id()
            .and_then(|u| u.to_string().parse::<u32>().ok())
            .and_then(|uid| passwd_users.get(&uid).cloned())
            .unwrap_or_else(|| "unknown".to_string());
        let cmd = if !proc_.cmd().is_empty() {
            proc_.cmd().join(" ")
        } else if let Some(exe) = proc_.exe() {
            if !exe.as_os_str().is_empty() {
                exe.display().to_string()
            } else {
                proc_.name().to_string()
            }
        } else {
            proc_.name().to_string()
        };
        let cwd = proc_
            .cwd()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let start_time = proc_.start_time();

        let mem_kb = proc_.memory() as f64;
        let mem_pct = if total_mem_kb > 0.0 {
            (mem_kb / total_mem_kb) * 100.0
        } else {
            0.0
        };
        let cpu = proc_.cpu_usage() as f64;

        out.push(ProcInfo {
            pid: pid_i32,
            user,
            cpu,
            mem_pct,
            mem_mb: mem_kb / 1024.0,
            command: cmd,
            cwd,
            start_time,
        });
    }

    // Filters
    if let Some(u) = &q.proc_user {
        out.retain(|p| p.user == *u);
    }
    if let Some(r) = &q.proc_regex {
        if let Ok(re) = RegexBuilder::new(r).size_limit(1 << 16).build() {
            out.retain(|p| re.is_match(&p.command));
        }
    }

    // Sorting
    let sort = q.proc_sort.as_deref().unwrap_or("cpu");
    let desc = q.proc_order.as_deref().unwrap_or("desc") != "asc";
    out.sort_by(|a, b| match sort {
        "mem" => a
            .mem_pct
            .partial_cmp(&b.mem_pct)
            .unwrap_or(std::cmp::Ordering::Equal),
        "pid" => a.pid.cmp(&b.pid),
        "user" => a.user.cmp(&b.user),
        "command" => a.command.cmp(&b.command),
        "start" => a.start_time.cmp(&b.start_time),
        _ => a
            .cpu
            .partial_cmp(&b.cpu)
            .unwrap_or(std::cmp::Ordering::Equal),
    });
    if desc {
        out.reverse();
    }

    // Pagination / top
    if let Some(top) = q.proc_top {
        if out.len() > top {
            out.truncate(top);
        }
    } else if let (Some(page), Some(page_size)) = (q.proc_page, q.proc_page_size) {
        let start = page.saturating_mul(page_size);
        out = out.into_iter().skip(start).take(page_size).collect();
    } else if out.len() > 20 {
        out.truncate(20);
    }

    out.into_iter()
        .map(|p| {
            serde_json::json!({
                "pid": p.pid,
                "user": p.user,
                "cpu": p.cpu,
                "memory": p.mem_pct,
                "mem_mb": p.mem_mb,
                "command": p.command,
                "cwd": p.cwd,
                "start_time": p.start_time,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- per-core CPU ---

    #[test]
    fn test_per_core_busy_percent_computes_and_clamps() {
        let a = [
            CpuSnap {
                idle: 100,
                total: 200,
            },
            CpuSnap {
                idle: 100,
                total: 200,
            },
        ];
        let b = [
            CpuSnap {
                idle: 150,
                total: 300,
            },
            // Idle went backwards (counter quirk): clamps to 100% busy.
            CpuSnap {
                idle: 90,
                total: 300,
            },
        ];
        let busy = per_core_busy_percent(&a, &b);
        assert_eq!(busy.len(), 2);
        assert!((busy[0] - 50.0).abs() < 1e-9);
        assert!((busy[1] - 100.0).abs() < 1e-9);
    }

    #[test]
    fn test_per_core_busy_percent_zero_delta_reports_zero() {
        let snap = [CpuSnap {
            idle: 100,
            total: 200,
        }];
        assert_eq!(per_core_busy_percent(&snap, &snap), vec![0.0]);
    }

    #[tokio::test]
    async fn test_read_per_core_cpu_reuses_result_within_min_interval() {
        // Unique scope so the shared baseline map cannot collide with other
        // tests or scopes.
        let scope = "test-scope-min-interval";
        let first = read_per_core_cpu(scope).await.unwrap();
        let second = read_per_core_cpu(scope).await.unwrap();
        assert!(!first.is_empty());
        // The second poll lands well inside CPU_RATE_MIN_INTERVAL_MS and must
        // return the cached result unchanged.
        assert_eq!(first, second);
    }

    // --- base_device_name ---

    #[test]
    fn test_base_device_name_sda1() {
        assert_eq!(base_device_name("/dev/sda1"), "sda");
    }

    #[test]
    fn test_base_device_name_nvme() {
        assert_eq!(base_device_name("/dev/nvme0n1p3"), "nvme0n1");
    }

    #[test]
    fn test_base_device_name_mmcblk() {
        assert_eq!(base_device_name("/dev/mmcblk0p1"), "mmcblk0");
    }

    #[test]
    fn test_base_device_name_no_partition() {
        assert_eq!(base_device_name("/dev/sda"), "sda");
    }

    // --- calc_rate ---

    /// First call returns 0 bps (no previous sample).
    /// Second call with increased counters returns a positive rate.
    #[test]
    fn test_calc_rate_first_call_is_zero() {
        // Use a unique key to avoid interference with other tests.
        let (ra, rb) = calc_rate("test-first-call", 1000, 2000);
        assert_eq!(ra, 0, "first call must return 0 — no previous sample");
        assert_eq!(rb, 0);
    }

    #[test]
    fn test_calc_rate_second_call_positive() {
        calc_rate("test-second-call", 0, 0); // seed
                                             // Give dt_ms a chance to be > 0 by sleeping 5 ms.
        std::thread::sleep(std::time::Duration::from_millis(5));
        let (ra, rb) = calc_rate("test-second-call", 5000, 10000);
        assert!(ra > 0, "rate must be positive after counter increase");
        assert!(rb > 0);
    }

    // --- list_procs_sysinfo pagination ---

    /// Default cap: more than 20 processes in the list must be truncated to 20.
    #[test]
    fn test_proc_list_default_cap_20() {
        let sys = System::new();
        let q = ProcQuery::default();
        // total_mem_kb = 0 causes all mem_pct to be 0 — fine for this test.
        let result = list_procs_sysinfo(&sys, 0.0, &q);
        assert!(
            result.len() <= 20,
            "default query must cap output at 20 entries"
        );
    }

    /// proc_top=5 must limit output to at most 5 entries.
    #[test]
    fn test_proc_list_top_param() {
        let sys = System::new();
        let q = ProcQuery {
            proc_top: Some(5),
            ..Default::default()
        };
        let result = list_procs_sysinfo(&sys, 0.0, &q);
        assert!(result.len() <= 5, "proc_top=5 must cap output at 5");
    }

    /// proc_regex that matches nothing must return an empty list.
    #[test]
    fn test_proc_list_regex_no_match() {
        let mut sys = System::new();
        sys.refresh_processes();
        let q = ProcQuery {
            proc_regex: Some("ZZZNOMATCH_XYZXYZ_999".to_string()),
            ..Default::default()
        };
        let result = list_procs_sysinfo(&sys, 0.0, &q);
        assert!(result.is_empty(), "no-match regex must return empty list");
    }
}
