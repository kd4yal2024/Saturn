//! Offline, bounded benchmark of the production display FFT and quantizer.
//! No backend, native DSP channel, socket, radio device or RF output is opened.
use saturn_bridge::diagnostic_fft::{quantize_display_db, SpectrumFft};
use std::hint::black_box;
use std::time::{Duration, Instant};

fn thread_cpu_ns() -> u64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) },
        0
    );
    t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64
}

fn percentiles(values: &mut [f64]) -> String {
    values.sort_by(f64::total_cmp);
    let at = |p: f64| values[((values.len() as f64 * p).ceil() as usize).saturating_sub(1)];
    format!(
        "{{\"count\":{},\"p50\":{},\"p95\":{},\"p99\":{},\"max\":{}}}",
        values.len(),
        at(0.5),
        at(0.95),
        at(0.99),
        values[values.len() - 1]
    )
}

fn main() {
    let began = Instant::now();
    let mut results = Vec::new();
    for size in [1024, 4096, 8192, 16384] {
        let iq: Vec<f32> = (0..size)
            .flat_map(|n| {
                let p = std::f64::consts::TAU * n as f64 / 256.0;
                [
                    (0.15 * p.cos() + 0.05 * (3.0 * p).cos()) as f32,
                    (0.15 * p.sin() - 0.05 * (3.0 * p).sin()) as f32,
                ]
            })
            .collect();
        let mut fft = SpectrumFft::new(size);
        let mut db = vec![0.0; size];
        let mut codes = vec![0u8; size];
        let mut wall = Vec::with_capacity(512);
        let mut cpu = Vec::with_capacity(512);
        for iteration in 0..520 {
            assert!(
                began.elapsed() < Duration::from_secs(20),
                "offline benchmark time budget exceeded"
            );
            let start = Instant::now();
            let cpu_start = thread_cpu_ns();
            fft.transform_db(black_box(&iq), &mut db);
            for (out, value) in codes.iter_mut().zip(&db) {
                *out = quantize_display_db(*value);
            }
            black_box(&codes);
            let cpu_ns = thread_cpu_ns() - cpu_start;
            let elapsed = start.elapsed();
            if iteration >= 8 {
                wall.push(elapsed.as_secs_f64() * 1000.0);
                cpu.push(cpu_ns as f64 / 1e6);
            }
        }
        results.push(format!(
            "{{\"fftSize\":{size},\"wallMs\":{},\"threadCpuMs\":{}}}",
            percentiles(&mut wall),
            percentiles(&mut cpu)
        ));
    }
    let governor = std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
        .unwrap_or_else(|_| "unavailable".into());
    println!("{{\"format\":\"saturn-offline-display-fft-bench-v1\",\"architecture\":\"{}\",\"governor\":{:?},\"warmupPerSize\":8,\"seconds\":{},\"scope\":\"production SpectrumFft plus u8 quantizer; preallocated buffers; includes clock-call overhead; excludes acquisition, ring snapshot, allocation/framing, TLS and scheduling load; synthetic IQ only\",\"hardwareAccess\":false,\"results\":[{}]}}",std::env::consts::ARCH,governor.trim(),began.elapsed().as_secs_f64(),results.join(","));
}
