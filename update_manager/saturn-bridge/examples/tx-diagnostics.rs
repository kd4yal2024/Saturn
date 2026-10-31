//! Offline only: reads bounded .satdiag capture files, never devices or RF.
use saturn_bridge::signal_diagnostics::{read_record, report_json};
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        return Err("usage: tx-diagnostics TONE1_HZ TONE2_HZ CAPTURE.satdiag [CAPTURE.satdiag ...]; signed tones for IQ, positive for PCM".into());
    }
    let tones = [args[0].parse::<f64>()?, args[1].parse::<f64>()?];
    let mut reports = Vec::new();
    for name in &args[2..] {
        if !std::fs::symlink_metadata(name)?.is_file() {
            return Err("ordinary capture files only; no symlinks or devices".into());
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(name)?;
        if !file.metadata()?.is_file() {
            return Err("ordinary capture files only".into());
        }
        reports.push(report_json(&read_record(file)?, tones));
    }
    println!("[{}]", reports.join(","));
    Ok(())
}
