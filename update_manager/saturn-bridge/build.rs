use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    emit_build_provenance();
    println!("cargo:rerun-if-env-changed=SATURN_BRIDGE_SOURCE_PIN");
    let source_pin = env::var("SATURN_BRIDGE_SOURCE_PIN")
        .ok()
        .filter(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .unwrap_or_else(|| "unverified".into());
    println!("cargo:rustc-env=SATURN_BRIDGE_SOURCE_PIN={source_pin}");
    println!(
        "cargo:rustc-env=SATURN_BRIDGE_WDSP_ARCHIVE_SHA256={}",
        native_digest("libwdsp.a")
    );
    println!(
        "cargo:rustc-env=SATURN_BRIDGE_WDSP_HEADER_SHA256={}",
        native_digest("wdsp.h")
    );
    println!("cargo:rerun-if-env-changed=SATURN_BRIDGE_STUB_NATIVE");
    println!("cargo:rustc-check-cfg=cfg(wdsp_has_rnnr_sbnr)");
    println!("cargo:rustc-check-cfg=cfg(wdsp_has_phrot_auto)");
    println!("cargo:rustc-check-cfg=cfg(wdsp_has_wbfm)");
    println!("cargo:rustc-check-cfg=cfg(saturn_bridge_stub_native)");
    if env::var("SATURN_BRIDGE_STUB_NATIVE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
    {
        println!("cargo:rustc-cfg=saturn_bridge_stub_native");
        build_stub_native();
        return;
    }

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    println!("cargo:rerun-if-env-changed=SATURN_WDSP_DIR");
    if let Ok(wdsp_dir) = env::var("SATURN_WDSP_DIR") {
        if env::var_os("CARGO_FEATURE_MANAGED_PURESIGNAL").is_some() {
            build_ps_observer(Path::new(&wdsp_dir));
        }
        link_wdsp_dir(PathBuf::from(wdsp_dir));
        return;
    }
    assert!(
        env::var_os("CARGO_FEATURE_MANAGED_PURESIGNAL").is_none(),
        "managed-puresignal requires an explicit reviewed SATURN_WDSP_DIR; no native fallback"
    );

    println!("cargo:rerun-if-env-changed=SATURN_PIHPSDR_DIR");
    let pihpsdr_dir = env::var("SATURN_PIHPSDR_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("../../../pihpsdr"));
    let wdsp_dir = pihpsdr_dir.join("wdsp");
    let libwdsp = wdsp_dir.join("libwdsp.a");
    let rnnoise_dir = pihpsdr_dir.join("rnnoise");
    let specbleach_dir = pihpsdr_dir.join("libspecbleach");

    if wdsp_has_rnnr_sbnr(&libwdsp) {
        println!("cargo:rustc-cfg=wdsp_has_rnnr_sbnr");
    }
    if wdsp_has_symbol(&libwdsp, "SetTXAPHROTAutoMode") {
        println!("cargo:rustc-cfg=wdsp_has_phrot_auto");
    }
    if wdsp_has_wbfm(&libwdsp) {
        println!("cargo:rustc-cfg=wdsp_has_wbfm");
    }

    if !libwdsp.exists() {
        panic!("WDSP static library not found at {}", libwdsp.display());
    }
    if !rnnoise_dir.join("librnnoise.a").exists() {
        panic!(
            "rnnoise static library not found at {}",
            rnnoise_dir.join("librnnoise.a").display()
        );
    }
    if !specbleach_dir.join("libspecbleach.a").exists() {
        panic!(
            "specbleach static library not found at {}",
            specbleach_dir.join("libspecbleach.a").display()
        );
    }

    println!("cargo:rerun-if-changed={}", libwdsp.display());
    println!(
        "cargo:rerun-if-changed={}",
        wdsp_dir.join("wdsp.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        rnnoise_dir.join("librnnoise.a").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        specbleach_dir.join("libspecbleach.a").display()
    );
    println!("cargo:rustc-link-search=native={}", wdsp_dir.display());
    println!("cargo:rustc-link-search=native={}", rnnoise_dir.display());
    println!(
        "cargo:rustc-link-search=native={}",
        specbleach_dir.display()
    );
    println!("cargo:rustc-link-lib=static=wdsp");
    println!("cargo:rustc-link-lib=static=specbleach");
    println!("cargo:rustc-link-lib=static=rnnoise");
    println!("cargo:rustc-link-lib=fftw3");
    println!("cargo:rustc-link-lib=fftw3f");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-lib=pthread");
}

fn build_ps_observer(wdsp: &Path) {
    println!("cargo:rerun-if-changed=src/wdsp/ps-native-header-pins.txt");
    for line in include_str!("src/wdsp/ps-native-header-pins.txt").lines() {
        let (expected, name) = line.split_once("  ").expect("header pin syntax");
        assert_eq!(
            native_digest(name),
            expected,
            "unreviewed private PS header {name}"
        );
    }
    assert_eq!(
        native_digest("wdsp.h"),
        "3dde5a9bfafe7e87f76684eeaab7da22fd5d421c08a8a725b0e2ec3cbd5416af",
        "unreviewed PS header"
    );
    let digest = native_digest("libwdsp.a");
    assert!(
        matches!(
            digest.as_str(),
            "8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248"
                | "f5f21c7cb76df1add17647c15a700f564a7f045e930e84be870333fa6e74973e"
        ),
        "unreviewed PS archive"
    );
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    // CALCC's controls are opaque in public wdsp.h. Derive the layout from the
    // exact linked source, never an independently hand-maintained offset table.
    assert_eq!(
        native_digest("calcc.c"),
        "1aa8470294640e7109824db7cc41786a02e21a5f0b9cf54bbbcfaf2df190ab66",
        "unreviewed native measurement control layout"
    );
    println!("cargo:rerun-if-changed={}", wdsp.join("calcc.c").display());
    let calcc = std::fs::read_to_string(wdsp.join("calcc.c")).expect("pinned CALCC source");
    let begin = calcc
        .find("typedef struct _psCollection* PSCOLLECTION;")
        .expect("CALCC prelude");
    let terminator = "} calcc, * CALCC;";
    let end = calcc[begin..]
        .find(terminator)
        .expect("CALCC declaration end")
        + begin
        + terminator.len();
    std::fs::write(out.join("saturn-calcc-layout.h"), &calcc[begin..end])
        .expect("CALCC layout extraction");
    let source = "src/wdsp/ps_observer.c";
    println!("cargo:rerun-if-changed={source}");
    println!("cargo:rerun-if-changed=src/wdsp/native_shutdown.c");
    println!("cargo:rerun-if-changed=../docs/puresignal/native-abi.h");
    println!("cargo:rerun-if-env-changed=SATURN_ARM_SYSROOT");
    let mut compiler = Command::new("cc");
    if env::var("TARGET") != env::var("HOST") {
        assert_eq!(env::var("TARGET").unwrap(), "aarch64-unknown-linux-gnu");
        let sysroot = PathBuf::from(
            env::var_os("SATURN_ARM_SYSROOT").expect("explicit ARM sysroot required"),
        );
        compiler = Command::new("/usr/bin/clang");
        compiler
            .arg("--target=aarch64-linux-gnu")
            .arg(format!("--sysroot={}", sysroot.display()))
            .arg("-isystem")
            .arg(sysroot.join("usr/include/aarch64-linux-gnu"));
    }
    let status = compiler
        .args(["-std=gnu11", "-D_GNU_SOURCE", "-O2", "-Werror", "-I"])
        .arg(wdsp)
        .arg("-I")
        .arg(&out)
        .args(["-c", source, "-o"])
        .arg(out.join("observer.o"))
        .status()
        .expect("PS observer compiler");
    assert!(status.success(), "matched PS observer compile failed");
    assert!(Command::new("ar")
        .arg("rcs")
        .arg(out.join("libsaturn_ps_observer.a"))
        .arg(out.join("observer.o"))
        .status()
        .unwrap()
        .success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=saturn_ps_observer");
    println!("cargo:rustc-link-arg=-Wl,--wrap=_beginthread");
    println!("cargo:rustc-link-arg=-Wl,--wrap=CloseChannel");
}

fn native_digest(name: &str) -> String {
    // A stub build must never inherit the identity of a real archive merely
    // because SATURN_WDSP_DIR was left in the environment.
    if env::var("SATURN_BRIDGE_STUB_NATIVE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
    {
        return "stub".into();
    }
    let Some(directory) = env::var_os("SATURN_WDSP_DIR") else {
        return "unverified".into();
    };
    let path = PathBuf::from(directory).join(name);
    let Ok(output) = Command::new("sha256sum").arg(&path).output() else {
        return "unverified".into();
    };
    if !output.status.success() {
        return "unverified".into();
    }
    println!("cargo:rerun-if-changed={}", path.display());
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("unverified")
        .into()
}

fn emit_build_provenance() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    let repo_dir = manifest_dir.join("../..");
    let git_sha = env::var("SATURN_BUILD_COMMIT")
        .ok()
        .filter(|value| value.len() == 40 && value.chars().all(|ch| ch.is_ascii_hexdigit()))
        .or_else(|| git_value(&repo_dir, &["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());
    let git_dirty = env::var("SATURN_BUILD_DIRTY")
        .ok()
        .and_then(|value| match value.as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        })
        .unwrap_or_else(|| {
            git_value(&repo_dir, &["status", "--porcelain"])
                .map(|value| !value.is_empty())
                .unwrap_or(false)
        });
    let wdsp_flavor =
        env::var("SATURN_BRIDGE_WDSP_FLAVOR").unwrap_or_else(|_| "unknown".to_string());
    let wdsp_commit =
        env::var("SATURN_BRIDGE_WDSP_COMMIT").unwrap_or_else(|_| "unknown".to_string());
    let target_cpu = env::var("SATURN_BRIDGE_TARGET_CPU").unwrap_or_else(|_| "generic".to_string());

    println!("cargo:rerun-if-env-changed=SATURN_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=SATURN_BUILD_DIRTY");
    println!("cargo:rerun-if-env-changed=SATURN_BRIDGE_WDSP_FLAVOR");
    println!("cargo:rerun-if-env-changed=SATURN_BRIDGE_WDSP_COMMIT");
    println!("cargo:rerun-if-env-changed=SATURN_BRIDGE_TARGET_CPU");
    println!("cargo:rustc-env=SATURN_BRIDGE_GIT_SHA={git_sha}");
    println!("cargo:rustc-env=SATURN_BRIDGE_GIT_DIRTY={git_dirty}");
    println!("cargo:rustc-env=SATURN_BRIDGE_WDSP_FLAVOR={wdsp_flavor}");
    println!("cargo:rustc-env=SATURN_BRIDGE_WDSP_COMMIT={wdsp_commit}");
    println!("cargo:rustc-env=SATURN_BRIDGE_TARGET_CPU={target_cpu}");
}

fn git_value(repo_dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn link_wdsp_dir(wdsp_dir: PathBuf) {
    let libwdsp = wdsp_dir.join("libwdsp.a");
    if !libwdsp.exists() {
        panic!("WDSP static library not found at {}", libwdsp.display());
    }

    let companion_dirs = if wdsp_has_rnnr_sbnr(&libwdsp) {
        println!("cargo:rustc-cfg=wdsp_has_rnnr_sbnr");
        Some((
            wdsp_companion_dir(&wdsp_dir, "SATURN_RNNOISE_DIR", "rnnoise", "librnnoise.a"),
            wdsp_companion_dir(
                &wdsp_dir,
                "SATURN_SPECBLEACH_DIR",
                "libspecbleach",
                "libspecbleach.a",
            ),
        ))
    } else {
        None
    };
    if wdsp_has_symbol(&libwdsp, "SetTXAPHROTAutoMode") {
        println!("cargo:rustc-cfg=wdsp_has_phrot_auto");
    }
    if wdsp_has_wbfm(&libwdsp) {
        println!("cargo:rustc-cfg=wdsp_has_wbfm");
    }

    println!("cargo:rerun-if-changed={}", libwdsp.display());
    println!(
        "cargo:rerun-if-changed={}",
        wdsp_dir.join("comm.h").display()
    );
    println!("cargo:rustc-link-search=native={}", wdsp_dir.display());
    if let Some((rnnoise_dir, specbleach_dir)) = &companion_dirs {
        println!("cargo:rustc-link-search=native={}", rnnoise_dir.display());
        println!(
            "cargo:rustc-link-search=native={}",
            specbleach_dir.display()
        );
    }
    println!("cargo:rustc-link-lib=static=wdsp");
    if companion_dirs.is_some() {
        println!("cargo:rustc-link-lib=static=specbleach");
        println!("cargo:rustc-link-lib=static=rnnoise");
    }
    println!("cargo:rustc-link-lib=fftw3");
    println!("cargo:rustc-link-lib=fftw3f");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-lib=pthread");
}

fn wdsp_companion_dir(
    wdsp_dir: &Path,
    env_name: &str,
    sibling_name: &str,
    archive_name: &str,
) -> PathBuf {
    println!("cargo:rerun-if-env-changed={env_name}");
    let dir = env::var(env_name).map(PathBuf::from).unwrap_or_else(|_| {
        wdsp_dir
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(sibling_name)
    });
    let archive = dir.join(archive_name);
    if !archive.exists() {
        panic!(
            "WDSP companion library not found at {}; set {env_name} to its directory",
            archive.display()
        );
    }
    println!("cargo:rerun-if-changed={}", archive.display());
    dir
}

fn wdsp_has_rnnr_sbnr(libwdsp: &PathBuf) -> bool {
    let Ok(output) = Command::new("nm")
        .arg("-g")
        .arg("--defined-only")
        .arg(libwdsp)
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let symbols = String::from_utf8_lossy(&output.stdout);
    symbols.contains(" RNNRloadModel\n")
        && symbols.contains(" SetRXARNNRRun\n")
        && symbols.contains(" SetRXASBNRRun\n")
}

fn wdsp_has_symbol(libwdsp: &Path, symbol: &str) -> bool {
    let Ok(output) = Command::new("nm")
        .arg("-g")
        .arg("--defined-only")
        .arg(libwdsp)
        .output()
    else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout).contains(&format!(" {symbol}\n"))
}

fn wdsp_has_wbfm(libwdsp: &Path) -> bool {
    wdsp_has_symbol(libwdsp, "SetRXAWBFMdmph")
        && wdsp_has_symbol(libwdsp, "GetRXAWBFMStereoIndicator")
}

fn build_stub_native() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let source = manifest_dir.join("native-stubs/wdsp_stub.c");
    let object = out_dir.join("wdsp_stub.o");
    let archive = out_dir.join("libsaturn_bridge_wdsp_stub.a");
    let cc = env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let ar = env::var("AR").unwrap_or_else(|_| "ar".to_string());

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rustc-cfg=wdsp_has_phrot_auto");
    println!("cargo:rustc-cfg=wdsp_has_wbfm");

    let cc_status = Command::new(&cc)
        .arg("-std=c99")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-c")
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {cc}: {error}"));
    if !cc_status.success() {
        panic!("failed to compile native stub {}", source.display());
    }

    let ar_status = Command::new(&ar)
        .arg("crs")
        .arg(&archive)
        .arg(&object)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {ar}: {error}"));
    if !ar_status.success() {
        panic!("failed to archive native stub {}", archive.display());
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=saturn_bridge_wdsp_stub");
}
