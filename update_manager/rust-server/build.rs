use std::path::{Path, PathBuf};
use std::process::Command;

fn normalized_commit(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() == 40 && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(value.to_ascii_lowercase())
    } else {
        None
    }
}

fn git_output(repo_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn watch_git_identity(repo_root: &Path) {
    if let Some(path) = git_output(repo_root, &["rev-parse", "--git-path", "HEAD"]) {
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            repo_root.join(path)
        };
        println!("cargo:rerun-if-changed={}", path.display());
    }

    if let Some(reference) = git_output(repo_root, &["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git_output(repo_root, &["rev-parse", "--git-path", &reference]) {
            let path = PathBuf::from(path);
            let path = if path.is_absolute() {
                path
            } else {
                repo_root.join(path)
            };
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=SATURN_BUILD_COMMIT");

    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set"));
    let repo_root = manifest_dir.join("../..");
    watch_git_identity(&repo_root);

    let commit = match std::env::var("SATURN_BUILD_COMMIT") {
        Ok(value) => normalized_commit(&value).unwrap_or_else(|| {
            panic!("SATURN_BUILD_COMMIT must be a full 40-character Git commit")
        }),
        Err(_) => git_output(&repo_root, &["rev-parse", "HEAD"])
            .and_then(|value| normalized_commit(&value))
            .unwrap_or_else(|| "unknown".to_string()),
    };

    println!("cargo:rustc-env=SATURN_BUILD_COMMIT={commit}");
}
