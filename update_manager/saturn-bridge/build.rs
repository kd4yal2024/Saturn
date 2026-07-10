use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SATURN_BRIDGE_STUB_NATIVE");
    println!("cargo:rustc-check-cfg=cfg(wdsp_has_rnnr_sbnr)");
    if env::var("SATURN_BRIDGE_STUB_NATIVE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
    {
        build_stub_native();
        return;
    }

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    println!("cargo:rerun-if-env-changed=SATURN_WDSP_DIR");
    if let Ok(wdsp_dir) = env::var("SATURN_WDSP_DIR") {
        link_wdsp_dir(PathBuf::from(wdsp_dir));
        return;
    }

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

fn link_wdsp_dir(wdsp_dir: PathBuf) {
    let libwdsp = wdsp_dir.join("libwdsp.a");
    if !libwdsp.exists() {
        panic!("WDSP static library not found at {}", libwdsp.display());
    }

    if wdsp_has_rnnr_sbnr(&libwdsp) {
        println!("cargo:rustc-cfg=wdsp_has_rnnr_sbnr");
    }

    println!("cargo:rerun-if-changed={}", libwdsp.display());
    println!(
        "cargo:rerun-if-changed={}",
        wdsp_dir.join("comm.h").display()
    );
    println!("cargo:rustc-link-search=native={}", wdsp_dir.display());
    println!("cargo:rustc-link-lib=static=wdsp");
    println!("cargo:rustc-link-lib=fftw3");
    println!("cargo:rustc-link-lib=fftw3f");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-lib=pthread");
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
