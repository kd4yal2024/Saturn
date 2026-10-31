//! Optional finite raw evidence, written only by the noncritical FFT worker.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
pub(super) struct Archive {
    directory: PathBuf,
    saved: usize,
    generations: BTreeMap<u64, (usize, u64)>,
    failed: bool,
}
// Two initial windows plus later evidence during a 45-second attended test.
// Selection uses original sample-arrival time, never worker publication time.
const CAPTURE_AFTER_NS: [u64; 4] = [0, 1_000_000_000, 15_000_000_000, 30_000_000_000];
impl Archive {
    pub(super) fn from_env() -> Option<Self> {
        let path = std::env::var_os("SATURN_RF_MONITOR_CAPTURE_DIRECTORY")?;
        match Self::create_process(Path::new(&path)) {
            Ok(a) => {
                eprintln!("saturn-bridge: RF paired archive={}", a.directory.display());
                Some(a)
            }
            Err(error) => {
                eprintln!("saturn-bridge: RF monitor raw archive unavailable: {error}");
                None
            }
        }
    }
    fn create_process(base: &Path) -> io::Result<Self> {
        crate::capture_directory::fresh_path(base).and_then(Self::create)
    }
    fn create(directory: PathBuf) -> io::Result<Self> {
        if !directory.is_absolute() || directory.parent().is_none() || directory.starts_with("/dev")
        {
            return Err(io::Error::other(
                "new explicit absolute archive directory required",
            ));
        }
        fs::create_dir(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        Ok(Self {
            directory,
            saved: 0,
            generations: BTreeMap::new(),
            failed: false,
        })
    }
    pub(super) fn record(&mut self, w: &Window, serial: u64, wire: &str) -> &'static str {
        if self.failed {
            return "archive failed";
        }
        if self.saved >= 32 {
            return "archive budget exhausted";
        }
        let generation = self
            .generations
            .entry(w.fence.generation)
            .or_insert((0, w.first_ns));
        let elapsed = w.first_ns.saturating_sub(generation.1);
        if generation.0 >= CAPTURE_AFTER_NS.len() || elapsed < CAPTURE_AFTER_NS[generation.0] {
            return "bounded archive interval";
        }
        let result = (|| -> io::Result<()> {
            let stem = format!("window-{serial}");
            let mut bytes = Vec::with_capacity(FFT_SIZE * 24);
            for i in 0..FFT_SIZE {
                for sample in [
                    w.reference[i * 2],
                    w.reference[i * 2 + 1],
                    w.rf[i * 2],
                    w.rf[i * 2 + 1],
                ] {
                    bytes.extend(sample.to_le_bytes());
                }
                bytes.extend(w.arrival_ns[i].to_le_bytes());
            }
            let mut raw = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(self.directory.join(format!("{stem}.pairs")))?;
            raw.write_all(&bytes)?;
            let digest = format!("{:x}", Sha256::digest(&bytes));
            let spectrum = wire
                .strip_prefix("saturn_ps_spectrum:")
                .and_then(|s| s.strip_suffix(';'))
                .ok_or_else(|| io::Error::other("invalid archive spectrum"))?;
            let mut metadata = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(self.directory.join(format!("{stem}.json")))?;
            write!(metadata,"{{\"format\":\"saturn-rf-paired-window-v1\",\"sourcePin\":\"{}\",\"wdspArchiveSha256\":\"{}\",\"rawFile\":\"{stem}.pairs\",\"rawSha256\":\"{digest}\",\"pairs\":{FFT_SIZE},\"recordBytes\":24,\"recordLayout\":\"ref_I:f32le,ref_Q:f32le,rf_I:f32le,rf_Q:f32le,original_bridge_arrival_ns:u64le\",\"scaling\":\"signed24 / 8388608; exact normalized float32, no AGC/resampling\",\"timeDomain\":\"original Bridge ingress, not FPGA/RF time\",\"spectrum\":{spectrum}}}\n",env!("SATURN_BRIDGE_SOURCE_PIN"),env!("SATURN_BRIDGE_WDSP_ARCHIVE_SHA256"))?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                generation.0 += 1;
                self.saved += 1;
                "raw pair saved"
            }
            Err(error) => {
                self.failed = true;
                eprintln!("saturn-bridge: RF monitor raw archive stopped: {error}");
                "archive failed"
            }
        }
    }
}
use std::os::unix::fs::OpenOptionsExt;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paired_archive_restart_uses_fresh_storage_and_preserves_old_evidence() {
        let fixture = crate::xdma_tx_radio::offline_fixture::DirectFixture::new();
        let base = fixture
            .register_path()
            .parent()
            .unwrap()
            .join("restart-pairs");
        let first = Archive::create_process(&base).unwrap();
        fs::write(first.directory.join("original.pairs"), b"retained").unwrap();
        let second = Archive::create_process(&base).unwrap();
        assert_ne!(first.directory, second.directory);
        assert_eq!(second.directory.parent(), Some(base.as_path()));
        assert_eq!(
            fs::read(first.directory.join("original.pairs")).unwrap(),
            b"retained"
        );
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn archive_is_new_private_exact_and_finite_with_original_times_and_source_identity() {
        let files = crate::xdma_tx_radio::offline_fixture::DirectFixture::new();
        let directory = files.register_path().parent().unwrap().join("rf-evidence");
        let mut archive = Archive::create(directory.clone()).unwrap();
        assert!(Archive::create(directory.clone()).is_err());
        let mut w = Window::default();
        w.pairs = FFT_SIZE;
        w.fence.generation = 1;
        w.first_ns = 10;
        w.reference[0] = 0.125;
        w.reference[1] = -0.25;
        w.rf[0] = 0.0625;
        w.rf[1] = -0.5;
        w.arrival_ns[0] = 123456;
        let wire =
            "saturn_ps_spectrum:{\"format\":\"synthetic-archive-test\",\"overload\":\"unknown\"};";
        assert_eq!(archive.record(&w, 1, wire), "raw pair saved");
        let raw = fs::read(directory.join("window-1.pairs")).unwrap();
        assert_eq!(raw.len(), FFT_SIZE * 24);
        assert_eq!(f32::from_le_bytes(raw[..4].try_into().unwrap()), 0.125);
        assert_eq!(f32::from_le_bytes(raw[8..12].try_into().unwrap()), 0.0625);
        assert_eq!(u64::from_le_bytes(raw[16..24].try_into().unwrap()), 123456);
        let metadata = fs::read_to_string(directory.join("window-1.json")).unwrap();
        assert!(metadata.contains("original Bridge ingress"));
        assert!(metadata.contains(env!("SATURN_BRIDGE_SOURCE_PIN")));
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(directory.join("window-1.pairs"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(archive.record(&w, 2, wire), "bounded archive interval");
        w.first_ns += 1_000_000_000;
        assert_eq!(archive.record(&w, 3, wire), "raw pair saved");
        w.first_ns += 1_000_000_000;
        assert_eq!(archive.record(&w, 4, wire), "bounded archive interval");
        w.first_ns = 15_000_000_010;
        assert_eq!(archive.record(&w, 6, wire), "raw pair saved");
        w.first_ns = 29_999_999_010;
        assert_eq!(archive.record(&w, 7, wire), "bounded archive interval");
        w.first_ns = 30_000_000_010;
        assert_eq!(archive.record(&w, 8, wire), "raw pair saved");
        w.first_ns += 1_000_000_000;
        assert_eq!(archive.record(&w, 9, wire), "bounded archive interval");
        archive.saved = 32;
        w.fence.generation = 2;
        assert_eq!(archive.record(&w, 5, wire), "archive budget exhausted");
        drop(archive);
        for serial in [1, 3, 6, 8] {
            for extension in ["pairs", "json"] {
                fs::remove_file(directory.join(format!("window-{serial}.{extension}"))).unwrap();
            }
        }
        fs::remove_dir(&directory).unwrap();
    }
}
