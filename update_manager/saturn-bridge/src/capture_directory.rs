//! Select fresh optional diagnostic storage on every process start. Existing
//! captures are immutable; restarting the radio must not require deleting them.
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static SERIAL: AtomicU64 = AtomicU64::new(0);
pub(crate) fn fresh_path(base: &Path) -> io::Result<PathBuf> {
    if !base.is_absolute()
        || ["/dev", "/proc", "/sys"]
            .iter()
            .any(|p| base.starts_with(p))
    {
        return Err(io::Error::other(
            "absolute diagnostic storage path required",
        ));
    }
    match fs::symlink_metadata(base) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(base.to_path_buf()),
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos();
            Ok(base.join(format!(
                "run-{}-{stamp}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            )))
        }
        Err(e) => Err(e),
        _ => Err(io::Error::other("diagnostic storage is not a directory")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_selects_new_storage_without_changing_old_evidence() {
        let base = std::env::temp_dir().join(format!(
            "saturn-capture-restart-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        assert_eq!(fresh_path(&base).unwrap(), base);
        fs::create_dir(&base).unwrap();
        fs::write(base.join("capture.json"), b"original").unwrap();
        let first = fresh_path(&base).unwrap();
        let second = fresh_path(&base).unwrap();
        assert_ne!(first, second);
        assert_eq!(first.parent(), Some(base.as_path()));
        fs::create_dir(first).unwrap();
        fs::create_dir(second).unwrap();
        assert_eq!(fs::read(base.join("capture.json")).unwrap(), b"original");
        assert!(fresh_path(Path::new("relative")).is_err());
        assert!(fresh_path(&base.join("capture.json")).is_err());
        fs::remove_dir_all(base).unwrap();
    }
}
