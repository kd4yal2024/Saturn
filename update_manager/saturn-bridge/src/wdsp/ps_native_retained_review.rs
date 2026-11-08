//! Explicit offline replay of pinned retained evidence. No radio, socket or RF
//! authority is constructed. Repetition is artificial replay, not RF testing.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn config(peak: f64) -> Config {
    Config {
        id: 1,
        route: 1,
        physical_adc: saturn_bridge::puresignal::OverloadSource::Adc1,
        enabled: true,
        automatic: false,
        hardware_peak: peak,
        requested_delay_seconds: 0.,
        settling_ns: 0,
        loop_delay_seconds: 0.,
        stale_after_ns: 150_000_000,
    }
}

#[test]
fn native_workers_keep_explicit_policy_after_cleanup_recreation() {
    std::thread::spawn(|| {
        unsafe extern "C" {
            fn fftw_set_timelimit(seconds: f64);
        }
        unsafe { fftw_set_timelimit(0.01) };
        let mut expected = crate::xdma_duc::allowed_cpu_ids().unwrap();
        let owner_cpu = *expected.last().unwrap();
        let worker_policy =
            super::super::native_worker_scheduling::WorkerPolicy::capture(true).unwrap();
        if expected.len() > 1 {
            expected.pop();
        }
        crate::xdma_duc::pin_current_thread(owner_cpu).unwrap();
        // Unprivileged inheritance regression. FIFO latency remains a separate
        // hardware qualification, not a claim made by this SCHED_BATCH test.
        let parameter = libc::sched_param { sched_priority: 0 };
        assert_eq!(
            unsafe { libc::sched_setscheduler(0, libc::SCHED_BATCH, &parameter) },
            0
        );
        let model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
        let mut native = FullNative::matched_with_worker_policy(&model, worker_policy).unwrap();
        let check = || {
            let mut found = std::collections::BTreeSet::new();
            for task in std::fs::read_dir("/proc/self/task").unwrap().flatten() {
                let name = std::fs::read_to_string(task.path().join("comm")).unwrap_or_default();
                if ["Wflush1", "PURESIGNAL", "Wchan1"].contains(&name.trim()) {
                    let tid: i32 = task.file_name().to_str().unwrap().parse().unwrap();
                    assert_eq!(unsafe { libc::sched_getscheduler(tid) }, libc::SCHED_OTHER);
                    let mut mask: libc::cpu_set_t = unsafe { std::mem::zeroed() };
                    assert_eq!(
                        unsafe {
                            libc::sched_getaffinity(tid, std::mem::size_of_val(&mask), &mut mask)
                        },
                        0
                    );
                    let cpus: Vec<_> = (0..libc::CPU_SETSIZE as usize)
                        .filter(|cpu| unsafe { libc::CPU_ISSET(*cpu, &mask) })
                        .collect();
                    assert_eq!(cpus, expected, "native worker {name}");
                    found.insert(name.trim().to_owned());
                }
            }
            assert_eq!(found.len(), 3, "actual named WDSP workers required");
            assert_eq!(crate::xdma_duc::current_scheduler().unwrap(), ("batch", 0));
            assert_eq!(crate::xdma_duc::allowed_cpu_ids().unwrap(), vec![owner_cpu]);
        };
        check();
        let mut cfg = config(0.67);
        cfg.enabled = false;
        native.configure(cfg).unwrap();
        for _ in 0..2 {
            native.begin_cleanup().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while native.cleanup_step().unwrap() != CleanupProgress::Complete {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            check();
        }
        assert_eq!(native.audit().recreations, 2);
    })
    .join()
    .unwrap();
}

fn exercise(label: &str, peak: f64, rows: Option<&[[f64; 4]]>) {
    let model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
    let mut native = FullNative::matched(&model).unwrap();
    native.configure(config(peak)).unwrap();
    let baseline = native.information().unwrap()[5];
    native.mox(true);
    let mut random = 0x53460004u32;
    let mut max_calibrations = baseline;
    let mut saw_correction = false;
    let mut saw_collect = false;
    let mut blocks = 0;
    for block in 0..512 {
        let mut tx = [0.; 2048];
        let mut rf = [0.; 2048];
        for pair in 0..1024 {
            let values = if let Some(rows) = rows {
                rows[(block * 1024 + pair) % rows.len()]
            } else {
                // SYNTHETIC positive control, not repaired or rescaled bench data.
                let mut uniform = || {
                    random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                    f64::from(random) / 4294967296.
                };
                let radius = 0.001 + 0.999 * uniform();
                let phase = std::f64::consts::TAU * uniform();
                let received = 0.5 * radius * (1. - 0.25 * radius * radius);
                let shifted = phase + 0.18 * radius * radius;
                [
                    peak * radius * phase.cos(),
                    peak * radius * phase.sin(),
                    received * shifted.cos(),
                    received * shifted.sin(),
                ]
            };
            tx[2 * pair..2 * pair + 2].copy_from_slice(&values[..2]);
            rf[2 * pair..2 * pair + 2].copy_from_slice(&values[2..]);
        }
        native.process(&mut tx, &mut rf).unwrap();
        // Continue native DSP while asynchronous fitting/correction can advance.
        if block % 2 == 0 {
            native.render(&[0.; 512]).unwrap();
        }
        let info = native.information().unwrap();
        max_calibrations = max_calibrations.max(info[5]);
        saw_correction |= info[14] != 0;
        saw_collect |= info[15] == 4;
        blocks += 1;
        std::thread::sleep(Duration::from_millis(1));
        if rows.is_none() && saw_correction {
            break;
        }
    }
    let audit = native.audit();
    native.begin_cleanup().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while native.cleanup_step().unwrap() != CleanupProgress::Complete {
        assert!(Instant::now() < deadline, "{label}: cleanup timeout");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        audit.exchanges > 0 && saw_collect,
        "nonvacuous native replay"
    );
    if rows.is_some() {
        assert_eq!(
            max_calibrations, baseline,
            "retained unequal tones unexpectedly fitted"
        );
        assert!(
            !saw_correction,
            "retained unequal tones unexpectedly corrected"
        );
    } else {
        assert!(
            max_calibrations > baseline && saw_correction,
            "positive control must really fit"
        );
    }
    eprintln!("retained_review label={label} peak={peak} blocks={blocks} saw_collect={saw_collect} calibrations_delta={} correction={saw_correction} exchanges={} cleanup=complete replay_only=true rf_qualified=false",
              max_calibrations-baseline,audit.exchanges);
}

#[test]
#[ignore = "serial pinned WDSP curve-lifetime contract; synthetic input, no RF or hardware"]
fn reviewed_wdsp_mox_only_retains_curve_and_cold_cleanup_discards_it() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe { fftw_set_timelimit(0.01) };
    let model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
    let mut native = FullNative::matched(&model).unwrap();
    let peak = 0.68292;
    native.configure(config(peak)).unwrap();
    native.mox(true); // Initial Single calibration request through current port.
    let mut random = 0x53460004u32;
    let mut fitted = false;
    for _ in 0..512 {
        let mut tx = [0.; 2048];
        let mut rf = [0.; 2048];
        for n in 0..1024 {
            let mut uniform = || {
                random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                f64::from(random) / 4294967296.
            };
            let radius = 0.001 + 0.999 * uniform();
            let phase = std::f64::consts::TAU * uniform();
            let received = 0.5 * radius * (1. - 0.25 * radius * radius);
            let shifted = phase + 0.18 * radius * radius;
            tx[2 * n] = peak * radius * phase.cos();
            tx[2 * n + 1] = peak * radius * phase.sin();
            rf[2 * n] = received * shifted.cos();
            rf[2 * n + 1] = received * shifted.sin();
        }
        native.process(&mut tx, &mut rf).unwrap();
        native.render(&[0.; 512]).unwrap();
        let info = native.info();
        if info[5] > 0 && info[14] == 1 && info[15] == 8 && (native.flags)() == 1 {
            fitted = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        fitted,
        "positive control must reach Single LSTAYON with real IQC"
    );
    let count = native.info()[5];
    // Native contract only: deliberately bypass the port's per-key mode
    // request. This does NOT assert that production warm release exists.
    for keyed in [false, true] {
        unsafe {
            SetPSMox(WDSP_TX_CHANNEL, i32::from(keyed));
        }
        for _ in 0..8 {
            native.process(&mut [0.; 2048], &mut [0.; 2048]).unwrap();
            native.render(&[0.; 512]).unwrap();
            let info = native.info();
            assert_eq!((info[5], info[14], info[15]), (count, 1, 8));
            assert_eq!((native.flags)(), 1, "loaded IQC without a pending swap");
        }
    }
    let before = native.audit().recreations;
    native.begin_cleanup().unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while native.cleanup_step().unwrap() != CleanupProgress::Complete {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(native.audit().recreations, before + 1);
    assert_eq!(native.info()[14], 0);
    assert_eq!((native.flags)(), 0);
    eprintln!("curve_lifetime native_mox_only_retained=true cold_cleanup_recreated=true production_warm_release=false synthetic=true physical_rf=false");
}

#[test]
#[ignore = "explicit pinned retained capture replay and synthetic positive control; no devices, sockets or RF"]
fn retained_feedback_collection_scaling_real_native() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe {
        fftw_set_timelimit(0.01);
    }
    let dir = PathBuf::from(
        std::env::var_os("SATURN_PS_RETAINED_PAIRS_DIR")
            .expect("explicit offline retained capture directory"),
    );
    for (name, expected) in [
        (
            "window-10.pairs",
            "d54ceb730d0e5045071bca0aafb5a4c99f3c981d75d168d7e47220bed7b9e4b3",
        ),
        (
            "window-53.pairs",
            "2988dd6f69c9d374e56d74054723f62ef8cb5f8dced0c21056bb5529e51b1b1a",
        ),
    ] {
        let path = dir.join(name);
        let meta = std::fs::symlink_metadata(&path).unwrap();
        assert!(meta.is_file() && meta.len() == 16384 * 24);
        let data = std::fs::read(path).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&data)),
            expected,
            "changed retained evidence"
        );
        let rows: Vec<[f64; 4]> = data
            .chunks_exact(24)
            .map(|r| {
                std::array::from_fn(|i| {
                    f64::from(f32::from_le_bytes(r[i * 4..i * 4 + 4].try_into().unwrap()))
                })
            })
            .collect();
        assert!(rows.iter().flatten().all(|v| v.is_finite()));
        for peak in [1., 0.68292] {
            exercise(name, peak, Some(&rows));
        }
    }
    exercise("SYNTHETIC full-range positive control", 0.68292, None);
}

#[test]
#[ignore = "offline candidate source experiment in the existing full native owner; not production wiring or RF"]
fn retained_review_balanced_postgen_with_full_native() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe { fftw_set_timelimit(0.01) };
    for mode in [DemodMode::Usb, DemodMode::Lsb] {
        let mut model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
        model.desired.mode = mode;
        model.desired.tx_mic_gain_db = -40.;
        model.desired.tx_eq_enabled = true;
        model.desired.tx_eq_bands = [6, -9, 9, -9, 9, -9, 9, -9, 9, -9, 9];
        model.desired.cfc_enabled = true;
        // Deliberately unsuitable voice passband: proposed calibration source
        // must not depend on it, or quietly modify this committed setting.
        model.desired.tx_filter_low_hz = 900;
        model.desired.tx_filter_high_hz = 1800;
        let mut native = FullNative::matched(&model).unwrap();
        let mut cfg = config(0.68292);
        cfg.enabled = false;
        native.configure(cfg).unwrap();
        native.measurement_mic_gain(true).unwrap();
        // Exercise the same preparation method selected by the shared owner.
        // Pinned TXA.c places gen1 after voice DSP and before PureSignal IQC.
        let sign = if mode == DemodMode::Lsb { -1. } else { 1. };
        native
            .diagnostic_stimulus(crate::tx_thread::feedback_owner::Stimulus::NativeTwoTone)
            .unwrap();
        assert_eq!(native.render(&[0.1; 512]), Err(Fault::InvalidConfig));
        let mut retained = Vec::new();
        for block in 0..96 {
            native.measurement_only_guard().unwrap();
            let iq = native.render(&[0.; 512]).unwrap();
            if block >= 80 {
                retained.extend(iq);
            }
        }
        let amplitude = |hz: f64| {
            let mut total = [0.; 2];
            for (n, pair) in retained.chunks_exact(2).enumerate() {
                let phase = std::f64::consts::TAU * hz * n as f64 / 192000.;
                total[0] += f64::from(pair[0]) * phase.cos() + f64::from(pair[1]) * phase.sin();
                total[1] += f64::from(pair[1]) * phase.cos() - f64::from(pair[0]) * phase.sin();
            }
            total[0].hypot(total[1]) / (retained.len() / 2) as f64
        };
        let a = amplitude(-sign * 700.);
        let b = amplitude(-sign * 1900.);
        assert!(a > 0.47 && b > 0.47 && (a / b - 1.).abs() < 0.005);
        let mut counts = [0u32; 16];
        for pair in retained.chunks_exact(2) {
            let env = f64::from(pair[0]).hypot(f64::from(pair[1]));
            assert!(env < 0.97, "bounded native source headroom");
            if env >= 0.005 {
                counts[((env * 16.) as usize).min(15)] += 1;
            }
        }
        assert!(
            counts.iter().all(|n| *n >= 256),
            "digital collection geometry: {counts:?}"
        );
        native.begin_cleanup().unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while native.cleanup_step().unwrap() != CleanupProgress::Complete {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(native.model.desired.tx_mic_gain_db, -40.);
        assert_eq!(native.model.desired.tx_eq_bands, model.desired.tx_eq_bands);
        assert_eq!(native.model.desired.tx_filter_low_hz, 900);
        assert_eq!(native.model.desired.tx_filter_high_hz, 1800);
        assert!(native.model.desired.cfc_enabled);
        assert!(!native.model.desired.two_tone_enabled);
        let mut after = 0f32;
        for _ in 0..96 {
            after = native
                .render(&[0.; 512])
                .unwrap()
                .iter()
                .fold(after, |p, s| p.max(s.abs()));
        }
        assert!(after < 1e-8, "generator must be gone after cleanup");
        eprintln!("offline_postgen_source mode={mode:?} tone_amplitudes={a:.8},{b:.8} counts={counts:?} normal_profile_restored=true zero_input_after_cleanup_peak={after} fitting=false rf_qualified=false production_preparation=true");
    }
}

// New retained-data experiment only. This neither changes the production
// profile nor silently updates the earlier unequal-tone negative controls.
#[test]
#[ignore = "explicit 20261019 G2 retained balanced-source replay; artificial repetition, no devices or RF"]
fn retained_balanced_g2_20261019_native_fit_review() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
        fn GetPSDisp(
            channel: i32,
            x: *mut f64,
            ym: *mut f64,
            yc: *mut f64,
            ys: *mut f64,
            xm: *mut f64,
            mc: *mut f64,
            xa: *mut f64,
            ac: *mut f64,
            ns: *mut i32,
            nc: *mut i32,
            phase_reference: *mut f64,
        );
    }
    unsafe { fftw_set_timelimit(0.01) };
    let dir = PathBuf::from(
        std::env::var_os("SATURN_PS_BALANCED_RETAINED_DIR")
            .expect("explicit original paired-window directory required"),
    );
    let out = PathBuf::from(
        std::env::var_os("SATURN_PS_BALANCED_REVIEW_OUTPUT")
            .expect("explicit offline review output required"),
    );
    assert!(out.is_dir());
    let path = dir.join("window-10.pairs");
    let stat = std::fs::symlink_metadata(&path).unwrap();
    assert!(stat.is_file() && stat.len() == 16384 * 24);
    let data = std::fs::read(path).unwrap();
    let digest = format!("{:x}", Sha256::digest(&data));
    assert_eq!(
        digest,
        "a75c47049a127156956f43ce400ee2648a3117086d83ee65cd20e6f693ffc6a8"
    );
    let metadata = std::fs::read(dir.join("window-10.json")).unwrap();
    // Pin the original metadata together with the original samples. Parsing
    // and contract validation are additionally performed by the Python checker.
    assert_eq!(
        format!("{:x}", Sha256::digest(&metadata)),
        "58abb75b33bb1370957cd8b6d796cbe7497dc56c01dcd2c49b108e04cc4b5441"
    );
    let rows: Vec<[f64; 4]> = data
        .chunks_exact(24)
        .map(|r| {
            std::array::from_fn(|i| {
                f64::from(f32::from_le_bytes(r[4 * i..4 * i + 4].try_into().unwrap()))
            })
        })
        .collect();
    assert!(rows.iter().flatten().all(|v| v.is_finite()));

    // Genuine native positive control is deliberately separate from measured
    // rows; it must not be represented as an improved or repaired G2 capture.
    exercise("SYNTHETIC positive control before G2 replay", 0.683, None);
    for peak in [1., 0.683, 0.66] {
        let model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
        let mut native = FullNative::matched(&model).unwrap();
        let actual_delay = native.configure(config(peak)).unwrap();
        assert_eq!(actual_delay, 0., "zero-delay replay hypothesis only");
        let before = native.information().unwrap();
        native.mox(true);
        let mut largest_info = before;
        let mut last_info = before;
        let mut correction = false;
        let mut collecting = false;
        let mut states = [0u64; 16];
        let mut blocks = 0;
        for block in 0..1024 {
            let mut tx = [0.; 2048];
            let mut rf = [0.; 2048];
            for pair in 0..1024 {
                // Original values, original paired ordering, no gain/phase/delay
                // rewriting. Wrapping is ARTIFICIAL replay, not fresh RF data.
                let row = rows[(block * 1024 + pair) % rows.len()];
                tx[2 * pair..2 * pair + 2].copy_from_slice(&row[..2]);
                rf[2 * pair..2 * pair + 2].copy_from_slice(&row[2..]);
            }
            native.process(&mut tx, &mut rf).unwrap();
            if block % 2 == 0 {
                native.render(&[0.; 512]).unwrap();
            }
            last_info = native.information().unwrap();
            if last_info[5] >= largest_info[5] {
                largest_info = last_info;
            }
            correction |= last_info[14] != 0;
            collecting |= last_info[15] == 4;
            if (0..16).contains(&last_info[15]) {
                states[last_info[15] as usize] += 1;
            }
            blocks = block + 1;
            if correction {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut measured: [Vec<f64>; 4] = std::array::from_fn(|_| vec![0.; 4096]);
        let mut curves: [Vec<f64>; 4] = std::array::from_fn(|_| vec![0.; 512]);
        let mut ns = 0;
        let mut nc = 0;
        let mut phase_reference = 0.;
        let [x, ym, yc, ys] = &mut measured;
        let [xm, mc, xa, ac] = &mut curves;
        unsafe {
            GetPSDisp(
                WDSP_TX_CHANNEL,
                x.as_mut_ptr(),
                ym.as_mut_ptr(),
                yc.as_mut_ptr(),
                ys.as_mut_ptr(),
                xm.as_mut_ptr(),
                mc.as_mut_ptr(),
                xa.as_mut_ptr(),
                ac.as_mut_ptr(),
                &mut ns,
                &mut nc,
                &mut phase_reference,
            );
        }
        assert!((0..=4096).contains(&ns) && (0..=512).contains(&nc));
        for v in &mut measured {
            v.truncate(ns as usize);
        }
        for v in &mut curves {
            v.truncate(nc as usize);
        }
        let finite = measured
            .iter()
            .chain(curves.iter())
            .flatten()
            .all(|v| v.is_finite());
        let audit = native.audit();
        native.begin_cleanup().unwrap();
        let cleanup_start = Instant::now();
        loop {
            match native.cleanup_step() {
                Ok(CleanupProgress::Complete) => break,
                Ok(_) if cleanup_start.elapsed() < Duration::from_secs(5) => {}
                other => panic!(
                    "G2 retained replay peak={peak} cleanup={other:?} info={:?}",
                    native.information()
                ),
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        // Every interpolated value is a pinned hexadecimal identifier,
        // finite numeric array, integer or boolean: no new JSON dependency.
        assert!(finite && phase_reference.is_finite());
        let result = format!(
            r#"{{
            "format":"saturn-retained-balanced-native-fit-review-v1",
            "captureSourcePin":"df2d15a77204e782e50e8fc0599346975ed09aa9a4a89b8adf8559f539b5f831",
            "rawSha256":"{digest}","replaySourcePin":"{}",
            "nativeArchiveSha256":"{}",
            "architecture":"{}","hardwarePeakHypothesis":{peak},
            "delaySecondsHypothesis":0.0,"measuredDelayQualified":false,
            "originalPairs":{},"blocksReplayed":{blocks},"artificialRepetition":true,
            "amplitudeOrPhaseRewritten":false,"sawCollecting":{collecting},
            "calibrationsDelta":{},"sawCorrection":{correction},
            "info":{last_info:?},"stateCounts":{states:?},"displayFinite":{finite},
            "measuredPoints":{measured:?},"correctionCurves":{curves:?},"phaseReferenceDegrees":{phase_reference},
            "nativeExchanges":{},"cleanupCompleted":true,
            "cleanupElapsedMs":{},
            "overload":"unknown","hostProfileChanged":false,
            "rfImdQualified":false,"liveFittingAuthorized":false,
            "limitations":["Replayed one fixed window repeatedly; not a new RF capture or improved RF spectrum.",
                "Zero delay is an experiment hypothesis, not a unique bench delay.",
                "Native coefficient acceptance does not establish physical RF improvement."]
        }}"#,
            env!("SATURN_BRIDGE_SOURCE_PIN"),
            env!("SATURN_BRIDGE_WDSP_ARCHIVE_SHA256"),
            std::env::consts::ARCH,
            rows.len(),
            largest_info[5] - before[5],
            audit.exchanges,
            cleanup_start.elapsed().as_millis()
        );
        let file = out.join(format!(
            "g2-native-fit-{}.json",
            peak.to_string().replace('.', "_")
        ));
        assert!(!file.exists());
        std::fs::write(file, result).unwrap();
        eprintln!("G2_balanced_retained peak={peak} blocks={blocks} info={last_info:?} fit_delta={} correcting={correction} display_finite={finite} cleanup=complete replay_only=true",largest_info[5]-before[5]);
        assert!(collecting && audit.exchanges > 0 && finite);
        if peak == 1. {
            assert!(
                !correction && largest_info[5] == before[5],
                "placeholder negative control"
            );
        } else if peak == 0.683 {
            // Preserve the observed REJECTION as a negative control. The
            // original optimistic experiment remains recorded as failed;
            // this assertion does not relabel that hypothesis as accepted.
            assert!(!correction && largest_info[5] == before[5]);
            assert_eq!(last_info[2], 2, "pinned cosine-phase NF_FIT_BAD rejection");
        } else {
            assert!(
                correction && largest_info[5] > before[5],
                "0.66 offline hypothesis must really fit, not merely collect"
            );
            assert_eq!(measured[0].len(), 4096);
            assert_eq!(curves[0].len(), 512);
        }
    }
}

#[test]
#[ignore = "explicit two independent 20261022 G2 captures; offline fit/output-headroom review only"]
fn retained_balanced_g2_20261022_single_candidate_review() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe { fftw_set_timelimit(0.01) };
    let dir = PathBuf::from(
        std::env::var_os("SATURN_PS_OCT22_PAIRS").expect("original October22 pairs required"),
    );
    let output = PathBuf::from(
        std::env::var_os("SATURN_PS_OCT22_REVIEW_OUTPUT")
            .expect("new offline output directory required"),
    );
    assert!(output.is_dir());
    for (name, raw_sha, meta_sha) in [
        (
            "window-10",
            "2232731d30e0c9ccbc277be84597d30790c7aa6b71ea3199ff096336dfc17132",
            "5950940520f1b3afe2a46c9f2d5ebd11349f77fa449bf667de94af877ce0282b",
        ),
        (
            "window-54",
            "e102b36530ba439278983bc4fcb4cddb29f26580e59eef1f5a63306c6501bc8b",
            "a06657e40b734016c4765cdfeb56592e24434f2f08d247b6226a7b3ab6395ad8",
        ),
    ] {
        let raw = std::fs::read(dir.join(format!("{name}.pairs"))).unwrap();
        let metadata = std::fs::read(dir.join(format!("{name}.json"))).unwrap();
        assert_eq!(raw.len(), 16384 * 24);
        assert_eq!(format!("{:x}", Sha256::digest(&raw)), raw_sha);
        assert_eq!(format!("{:x}", Sha256::digest(&metadata)), meta_sha);
        let rows: Vec<[f64; 4]> = raw
            .chunks_exact(24)
            .map(|r| {
                std::array::from_fn(|i| {
                    f64::from(f32::from_le_bytes(r[4 * i..4 * i + 4].try_into().unwrap()))
                })
            })
            .collect();
        assert!(rows.iter().flatten().all(|v| v.is_finite()));
        // 0.67 is a rounded scale hypothesis derived from returned envelope
        // ~0.64249 / commanded native envelope 0.96, NOT from LP700 watts.
        // This is not a full-drive calibration or authorization to use it live.
        for peak in [1., 0.683, 0.67, 0.66] {
            let mut model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
            model.desired.mode = crate::radio_model::DemodMode::Lsb;
            let mut native = FullNative::matched(&model).unwrap();
            native.configure(config(peak)).unwrap();
            native.measurement_mic_gain(true).unwrap();
            native
                .diagnostic_stimulus(crate::tx_thread::feedback_owner::Stimulus::NativeTwoTone)
                .unwrap();
            let baseline = native.information().unwrap()[5];
            native.mox(true);
            let mut info = native.information().unwrap();
            let mut corrected = false;
            let mut blocks = 0;
            for block in 0..1024 {
                let mut reference = [0.; 2048];
                let mut feedback = [0.; 2048];
                for pair in 0..1024 {
                    let row = rows[(block * 1024 + pair) % rows.len()];
                    reference[2 * pair..2 * pair + 2].copy_from_slice(&row[..2]);
                    feedback[2 * pair..2 * pair + 2].copy_from_slice(&row[2..]);
                }
                native.process(&mut reference, &mut feedback).unwrap();
                native.render(&[0.; 512]).unwrap();
                info = native.information().unwrap();
                blocks = block + 1;
                if info[14] != 0 && info[5] > baseline {
                    corrected = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            let mut iq_peak = 0f32;
            let mut rail_components = 0usize;
            // Continue the real TX DSP after coefficient installation. Fitting
            // acceptance alone does not establish corrected digital headroom.
            for _ in 0..96 {
                for value in native.render(&[0.; 512]).unwrap() {
                    assert!(value.is_finite());
                    iq_peak = iq_peak.max(value.abs());
                    rail_components += usize::from(value.abs() >= 1.);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            native.begin_cleanup().unwrap();
            let start = Instant::now();
            while native.cleanup_step().unwrap() != CleanupProgress::Complete {
                assert!(start.elapsed() < Duration::from_secs(5));
                std::thread::sleep(Duration::from_millis(1));
            }
            let result = format!(
                r#"{{"format":"saturn-oct22-offline-fit-review-v1","window":"{name}","rawSha256":"{raw_sha}","metadataSha256":"{meta_sha}","captureSourcePin":"983470bdf50861c298a8d78a377859521d6d25883caf27a0251487e663e4dc78","replaySourcePin":"{}","wdspArchiveSha256":"{}","architecture":"{}","hardwarePeakHypothesis":{peak},"delaySecondsHypothesis":0,"artificialRepetition":true,"samplesRewritten":false,"blocksReplayed":{blocks},"fitAccepted":{corrected},"info":{info:?},"postFitIqComponentPeak":{iq_peak},"postFitRailComponents":{rail_components},"cleanupCompleted":true,"cleanupElapsedMs":{},"overloadCoverage":"unknown","hostProfileChanged":false,"rfImdQualified":false,"liveCalibrationAuthorized":false}}"#,
                env!("SATURN_BRIDGE_SOURCE_PIN"),
                env!("SATURN_BRIDGE_WDSP_ARCHIVE_SHA256"),
                std::env::consts::ARCH,
                start.elapsed().as_millis()
            );
            let file = output.join(format!(
                "{name}-peak-{}.json",
                peak.to_string().replace('.', "_")
            ));
            assert!(!file.exists());
            std::fs::write(file, result).unwrap();
            eprintln!("oct22_offline_review window={name} peak={peak} fitted={corrected} info={info:?} postfit_iq_peak={iq_peak} rails={rail_components} cleanup=complete physical_rf=false");
            if peak == 1. {
                assert!(!corrected, "placeholder must remain a negative control");
            }
            if peak == 0.67 {
                assert!(corrected, "derived-scale hypothesis did not fit {name}");
                assert_eq!(rail_components, 0);
                assert!(iq_peak < 1., "corrected native output lacks headroom");
            }
        }
    }
}

#[test]
#[ignore = "offline path-specific admission and unsuitable-feedback controls; never RF"]
fn retained_balanced_g2_20261022_feedback_admission_review() {
    unsafe extern "C" {
        fn fftw_set_timelimit(seconds: f64);
    }
    unsafe { fftw_set_timelimit(0.01) };
    let dir = PathBuf::from(
        std::env::var_os("SATURN_PS_OCT22_PAIRS")
            .expect("explicit original paired window required"),
    );
    let bytes = std::fs::read(dir.join("window-10.pairs")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        "2232731d30e0c9ccbc277be84597d30790c7aa6b71ea3199ff096336dfc17132"
    );
    let rows: Vec<[f64; 4]> = bytes
        .chunks_exact(24)
        .map(|r| {
            std::array::from_fn(|i| {
                f64::from(f32::from_le_bytes(r[4 * i..4 * i + 4].try_into().unwrap()))
            })
        })
        .collect();
    let limits = saturn_bridge::puresignal::admission_proposal::DraftLimits {
        active_reference_fraction: 0.1,
        feedback_rms_floor: 0.01,
        component_limit: 0.95,
    };
    for case in [
        "original",
        "missing",
        "digital-rail",
        "uncorrelated-noise",
        "mistuned",
    ] {
        let model = RadioModel::new(6, 7_200_000, 0, 384, 24, 2048, true, 4096, true);
        let mut native = FullNative::matched(&model).unwrap();
        native.configure(config(0.67)).unwrap();
        native.mox(true);
        let mut rng = 0x53465031u32;
        let mut fitted = false;
        let mut rejected = 0;
        let mut min_tx_fraction = f64::INFINITY;
        let mut min_rf_rms = f64::INFINITY;
        for block in 0..1024 {
            let mut tx = [0.; 2048];
            let mut rf = [0.; 2048];
            for pair in 0..1024 {
                let index = block * 1024 + pair;
                let row = rows[index % rows.len()];
                tx[2 * pair..2 * pair + 2].copy_from_slice(&row[..2]);
                let value = match case {
                    "missing" => [0., 0.],
                    "digital-rail" => [0.99, row[3]],
                    "uncorrelated-noise" => std::array::from_fn(|_| {
                        rng ^= rng << 13;
                        rng ^= rng >> 17;
                        rng ^= rng << 5;
                        (f64::from(rng) / f64::from(u32::MAX) - 0.5) * 0.6
                    }),
                    "mistuned" => {
                        let phase = std::f64::consts::TAU * 100. * index as f64 / 192000.;
                        [
                            row[2] * phase.cos() - row[3] * phase.sin(),
                            row[2] * phase.sin() + row[3] * phase.cos(),
                        ]
                    }
                    _ => [row[2], row[3]],
                };
                rf[2 * pair..2 * pair + 2].copy_from_slice(&value);
            }
            let rms = |v: &[f64]| (v.iter().map(|x| x * x).sum::<f64>() / 1024.).sqrt();
            min_tx_fraction = min_tx_fraction.min(rms(&tx) / 0.67);
            min_rf_rms = min_rf_rms.min(rms(&rf));
            match limits.check_window(&tx, &rf, 0.67) {
                Err(fault) => {
                    assert_eq!(
                        fault,
                        if case == "missing" {
                            Fault::FeedbackLevel
                        } else {
                            Fault::SevereOverdrive
                        }
                    );
                    rejected += 1;
                    break;
                }
                Ok(()) => native.process(&mut tx, &mut rf).unwrap(),
            }
            native.render(&[0.; 512]).unwrap();
            let info = native.information().unwrap();
            fitted = info[5] > 0 && info[14] != 0;
            if fitted {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        native.begin_cleanup().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while native.cleanup_step().unwrap() != CleanupProgress::Complete {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        eprintln!("offline_oct22_admission case={case} accepted_fit={fitted} rejected_blocks={rejected} min_reference_rms_fraction={min_tx_fraction} min_rf_rms={min_rf_rms} overload=unknown cleanup=complete synthetic_fault_injection=true physical_rf=false");
        if case == "original" {
            assert!(fitted);
            assert!(min_tx_fraction > 0.6 && min_rf_rms > 0.2);
        } else {
            assert!(
                !fitted,
                "unsuitable {case} feedback unexpectedly installed correction"
            );
        }
    }
}
