use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
pub struct AtomicWriteOptions {
    pub mode: u32,
    pub preserve_last_good: bool,
}

impl AtomicWriteOptions {
    pub const fn state_file() -> Self {
        Self {
            mode: 0o640,
            preserve_last_good: true,
        }
    }

    pub const fn executable() -> Self {
        Self {
            mode: 0o755,
            preserve_last_good: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FaultPoint {
    None,
    AfterTempSync,
    AfterRename,
}

pub fn last_good_path(path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("state path has no valid filename: {}", path.display()))?;
    Ok(path.with_file_name(format!("{name}.last-good")))
}

pub async fn write_atomic(
    path: &Path,
    content: impl Into<Vec<u8>>,
    options: AtomicWriteOptions,
) -> Result<(), String> {
    let path = path.to_path_buf();
    let content = content.into();
    tokio::task::spawn_blocking(move || {
        write_atomic_sync(&path, &content, options, FaultPoint::None)
    })
    .await
    .map_err(|error| format!("atomic state writer task failed: {error}"))?
}

pub async fn write_json_atomic<T: Serialize>(
    path: &Path,
    value: &T,
    options: AtomicWriteOptions,
) -> Result<(), String> {
    let mut content = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("failed to serialize state document: {error}"))?;
    content.push(b'\n');
    write_atomic(path, content, options).await
}

fn write_atomic_sync(
    path: &Path,
    content: &[u8],
    options: AtomicWriteOptions,
    fault: FaultPoint,
) -> Result<(), String> {
    if options.mode & !0o7777 != 0 {
        return Err(format!("invalid state file mode: {:o}", options.mode));
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("state path has no parent directory: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "failed to create state directory {}: {error}",
            parent.display()
        )
    })?;
    let parent_metadata = fs::symlink_metadata(parent).map_err(|error| {
        format!(
            "failed to inspect state directory {}: {error}",
            parent.display()
        )
    })?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(format!(
            "state parent must be a real directory: {}",
            parent.display()
        ));
    }

    if options.preserve_last_good {
        let last_good = last_good_path(path)?;
        atomic_replace_sync(&last_good, content, options.mode, FaultPoint::None)?;
    }
    atomic_replace_sync(path, content, options.mode, fault)
}

fn atomic_replace_sync(
    path: &Path,
    content: &[u8],
    mode: u32,
    fault: FaultPoint,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("state path has no parent directory: {}", path.display()))?;
    let expected_owner = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "state target must be a regular file: {}",
                    path.display()
                ));
            }
            Some(metadata.uid())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "failed to inspect state target {}: {error}",
                path.display()
            ))
        }
    };

    let (temporary, mut file) = create_unique_temp(parent, path, mode)?;
    let result = (|| -> Result<(), String> {
        file.set_permissions(fs::Permissions::from_mode(mode))
            .map_err(|error| {
                format!(
                    "failed to set state temp mode {}: {error}",
                    temporary.display()
                )
            })?;
        let metadata = file.metadata().map_err(|error| {
            format!(
                "failed to inspect state temp file {}: {error}",
                temporary.display()
            )
        })?;
        if metadata.mode() & 0o7777 != mode {
            return Err(format!(
                "state temp mode mismatch for {}: expected {:04o}, got {:04o}",
                temporary.display(),
                mode,
                metadata.mode() & 0o7777
            ));
        }
        if let Some(owner) = expected_owner {
            if metadata.uid() != owner {
                return Err(format!(
                    "state owner mismatch for {}: existing uid {owner}, replacement uid {}",
                    path.display(),
                    metadata.uid()
                ));
            }
        }

        file.write_all(content)
            .and_then(|_| file.sync_all())
            .map_err(|error| {
                format!(
                    "failed to flush state temp file {}: {error}",
                    temporary.display()
                )
            })?;
        if fault == FaultPoint::AfterTempSync {
            return Err("injected state-write failure after temp-file sync".to_string());
        }
        drop(file);
        fs::rename(&temporary, path).map_err(|error| {
            format!(
                "failed to atomically activate state file {}: {error}",
                path.display()
            )
        })?;
        sync_directory(parent)?;
        if fault == FaultPoint::AfterRename {
            return Err("injected state-write failure after atomic rename".to_string());
        }
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn create_unique_temp(parent: &Path, target: &Path, mode: u32) -> Result<(PathBuf, File), String> {
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("state");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    for _ in 0..128 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{name}.{}-{nanos}-{sequence}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create state temp file {}: {error}",
                    path.display()
                ))
            }
        }
    }
    Err(format!(
        "could not allocate a unique state temp file beside {}",
        target.display()
    ))
}

fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            format!(
                "failed to flush state directory {}: {error}",
                path.display()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    fn temp_dir(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "saturn-state-store-{name}-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn atomic_write_sets_mode_owner_and_last_good() {
        let root = temp_dir("complete");
        let path = root.join("policy.json");
        write_atomic_sync(
            &path,
            br#"{"value":"old"}\n"#,
            AtomicWriteOptions::state_file(),
            FaultPoint::None,
        )
        .unwrap();
        let owner = fs::metadata(&path).unwrap().uid();

        write_atomic_sync(
            &path,
            br#"{"value":"new"}\n"#,
            AtomicWriteOptions::state_file(),
            FaultPoint::None,
        )
        .unwrap();

        assert_eq!(fs::read(&path).unwrap(), br#"{"value":"new"}\n"#);
        assert_eq!(
            fs::read(last_good_path(&path).unwrap()).unwrap(),
            br#"{"value":"new"}\n"#
        );
        let metadata = fs::metadata(&path).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o640);
        assert_eq!(metadata.uid(), owner);
        assert!(!fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".tmp")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fault_before_rename_exposes_complete_old_document() {
        let root = temp_dir("old");
        let path = root.join("state.json");
        fs::write(&path, b"old-document\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        let error = write_atomic_sync(
            &path,
            b"new-document\n",
            AtomicWriteOptions {
                mode: 0o640,
                preserve_last_good: false,
            },
            FaultPoint::AfterTempSync,
        )
        .unwrap_err();
        assert!(error.contains("injected"));
        assert_eq!(fs::read(&path).unwrap(), b"old-document\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fault_after_rename_exposes_complete_new_document() {
        let root = temp_dir("new");
        let path = root.join("state.json");
        fs::write(&path, b"old-document\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        let error = write_atomic_sync(
            &path,
            b"new-document\n",
            AtomicWriteOptions {
                mode: 0o640,
                preserve_last_good: false,
            },
            FaultPoint::AfterRename,
        )
        .unwrap_err();
        assert!(error.contains("injected"));
        assert_eq!(fs::read(&path).unwrap(), b"new-document\n");
        fs::remove_dir_all(root).unwrap();
    }
}
