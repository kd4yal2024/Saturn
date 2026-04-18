use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    let wdsp_dir = manifest_dir.join("../../../pihpsdr/wdsp");
    let libwdsp = wdsp_dir.join("libwdsp.a");
    let rnnoise_dir = manifest_dir.join("../../../pihpsdr/rnnoise");
    let specbleach_dir = manifest_dir.join("../../../pihpsdr/libspecbleach");

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
