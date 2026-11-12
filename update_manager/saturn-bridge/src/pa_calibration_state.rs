//! Host-owned PA preset, independent of receive state and TX authority.
//! Only committed model values are watched. No client draft, PTT, drive,
//! generator, PureSignal mode or output generation can be restored here.
use crate::{pa_calibration::PaCalibration, radio_model::RadioModel, sync_ext::MutexExt};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const FILE: &str = "pa-calibration-v1.txt";
const FORMAT: &str = "format=saturn-pa-calibration\nversion=1\nprofile=";
const MAX_BYTES: u64 = 1024;
const POLL: Duration = Duration::from_millis(250);
#[derive(Clone)]
pub(crate) struct Store {
    directory: PathBuf,
}
impl Store {
    pub(crate) fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
    fn path(&self) -> PathBuf {
        self.directory.join(FILE)
    }
    pub(crate) fn load(&self) -> io::Result<Option<PaCalibration>> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.path())
        {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_BYTES {
            return Err(invalid());
        }
        let mut text = String::new();
        file.take(MAX_BYTES + 1).read_to_string(&mut text)?;
        if text.len() as u64 > MAX_BYTES {
            return Err(invalid());
        }
        let profile = text
            .strip_prefix(FORMAT)
            .and_then(|v| v.strip_suffix('\n'))
            .and_then(PaCalibration::parse)
            .ok_or_else(invalid)?;
        Ok(Some(profile))
    }
    pub(crate) fn save(&self, profile: PaCalibration) -> io::Result<()> {
        if !profile.valid() {
            return Err(invalid());
        }
        // Unknown versions, malformed files and symlinks are retained for
        // diagnosis instead of being silently replaced by a default profile.
        self.load()?;
        fs::create_dir_all(&self.directory)?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temporary = self.directory.join(format!(
            ".{FILE}.{}.{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(format!("{FORMAT}{}\n", profile.wire()).as_bytes())?;
            file.sync_all()?;
            fs::rename(&temporary, self.path())?;
            fs::File::open(&self.directory)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
    pub(crate) fn start(self, model: Arc<Mutex<RadioModel>>) -> io::Result<Saver> {
        match self.load() {
            Ok(Some(profile)) => {
                model.lock_unpoisoned().desired.tx_pa_calibration = profile;
                eprintln!(
                    "saturn-bridge: restored host PA calibration from {}",
                    self.path().display()
                );
            }
            Ok(None) => {}
            Err(e) => eprintln!(
                "saturn-bridge: PA calibration preset {} not restored: {e}",
                self.path().display()
            ),
        }
        let baseline = model.lock_unpoisoned().desired.tx_pa_calibration;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = thread::Builder::new()
            .name("pa-preset-saver".into())
            .spawn(move || {
                let mut written = baseline;
                let mut error_at: Option<Instant> = None;
                loop {
                    let done = flag.load(Ordering::Acquire);
                    let current = model.lock_unpoisoned().desired.tx_pa_calibration;
                    if current != written {
                        match self.save(current) {
                            Ok(()) => {
                                written = current;
                                eprintln!(
                                    "saturn-bridge: saved host PA calibration to {}",
                                    self.path().display()
                                );
                            }
                            Err(e)
                                if error_at
                                    .is_none_or(|at| at.elapsed() >= Duration::from_secs(60)) =>
                            {
                                eprintln!(
                                    "saturn-bridge: PA calibration preset {} not saved: {e}",
                                    self.path().display()
                                );
                                error_at = Some(Instant::now());
                            }
                            Err(_) => {}
                        }
                    }
                    if done {
                        break;
                    }
                    thread::park_timeout(POLL);
                }
            })?;
        Ok(Saver {
            stop,
            handle: Some(handle),
        })
    }
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid or unsupported PA calibration preset",
    )
}
pub(crate) struct Saver {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}
impl Drop for Saver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, Arc<Mutex<RadioModel>>) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "saturn-pa-preset-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        (
            directory,
            Arc::new(Mutex::new(RadioModel::new(
                6, 7_200_000, 0, 384, 24, 2048, true, 4096, true,
            ))),
        )
    }
    #[test]
    fn committed_profile_survives_shutdown_restart_without_any_tx_state() {
        let (directory, model) = fixture();
        let store = Store::new(directory.clone());
        let saver = store.clone().start(model.clone()).unwrap();
        let profile = PaCalibration::Bands(crate::pa_calibration::G2_STARTING_GAINS);
        model.lock_unpoisoned().desired.tx_pa_calibration = profile;
        drop(saver);
        assert_eq!(store.load().unwrap(), Some(profile));
        let restored = Arc::new(Mutex::new(RadioModel::new(
            6, 3_899_000, 0, 384, 24, 2048, true, 4096, true,
        )));
        let before = restored.lock_unpoisoned().desired.clone();
        let restarted = store.clone().start(restored.clone()).unwrap();
        let actual = restored.lock_unpoisoned().desired.clone();
        assert_eq!(actual.tx_pa_calibration, profile);
        assert_eq!(actual.tx_drive, before.tx_drive);
        assert_eq!(actual.tx_enabled, before.tx_enabled);
        assert_eq!(actual.tx_phase, before.tx_phase);
        assert_eq!(actual.pure_signal_enabled, before.pure_signal_enabled);
        assert_eq!(actual.two_tone_enabled, before.two_tone_enabled);
        drop(restarted);
        let legacy = store.clone().start(restored.clone()).unwrap();
        restored.lock_unpoisoned().desired.tx_pa_calibration = PaCalibration::Legacy;
        drop(legacy);
        assert_eq!(store.load().unwrap(), Some(PaCalibration::Legacy));
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn unsupported_malformed_and_control_bearing_files_are_not_applied_or_replaced() {
        for content in [
            "format=saturn-pa-calibration\nversion=2\nprofile=legacy\n",
            "format=saturn-pa-calibration\nversion=1\nprofile=legacy\ntx_enabled=true\n",
            "invalid",
        ] {
            let (directory, model) = fixture();
            let store = Store::new(directory.clone());
            fs::write(store.path(), content).unwrap();
            let saver = store.clone().start(model.clone()).unwrap();
            assert_eq!(
                model.lock_unpoisoned().desired.tx_pa_calibration,
                PaCalibration::Legacy
            );
            model.lock_unpoisoned().desired.tx_pa_calibration =
                PaCalibration::Bands(crate::pa_calibration::G2_STARTING_GAINS);
            drop(saver);
            assert_eq!(fs::read_to_string(store.path()).unwrap(), content);
            fs::remove_dir_all(directory).unwrap();
        }
    }
    #[test]
    fn symlink_cannot_redirect_preset_reads_or_writes() {
        let (directory, model) = fixture();
        let store = Store::new(directory.clone());
        let other = directory.join("other");
        fs::write(&other, "keep").unwrap();
        std::os::unix::fs::symlink(&other, store.path()).unwrap();
        let saver = store.start(model.clone()).unwrap();
        model.lock_unpoisoned().desired.tx_pa_calibration =
            PaCalibration::Bands(crate::pa_calibration::G2_STARTING_GAINS);
        drop(saver);
        assert_eq!(fs::read_to_string(other).unwrap(), "keep");
        fs::remove_dir_all(directory).unwrap();
    }
}
