//! Explicit diagnostic transaction. No native, RF, file I/O or JSON encoding
//! runs in the TX owner's completion step. Evidence is exported by a later
//! authenticated control request, only after completed backend/native cleanup.
use super::ps_owner_loop::Cancel;
use crate::{sync_ext::MutexExt, tci::ps_authorization::OperatorIngress};
use saturn_bridge::puresignal::{
    measurement::MeasurementRecorder,
    tx_lifecycle::{CooperativeNative, Phase, TxAuthority, TxLifecycle},
    Fault,
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub(crate) const MAX_DURATION_MS: u64 = 1_000;
pub(crate) const MAX_DRIVE_WATTS: u8 = 5;
pub(crate) const CAPACITY_BLOCKS: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PhaseRecord {
    Idle,
    Queued,
    Running,
    Complete,
    Exporting,
    Exported,
    Rejected,
    Blocked,
}
struct Slot {
    id: u64,
    phase: PhaseRecord,
    records: Option<MeasurementRecorder>,
    export_attempt: u64,
}
pub(crate) struct Host {
    directory: File,
    display_path: PathBuf,
    pub(crate) frequency_hz: u32,
    slot: Mutex<Slot>,
}
impl Host {
    pub(crate) fn status(&self) -> String {
        let slot = self.slot.lock_unpoisoned();
        format!("{},status,{:?}", slot.id, slot.phase)
    }
    pub(crate) fn open(path: &Path, frequency_hz: u32) -> io::Result<Arc<Self>> {
        if !path.is_absolute()
            || path
                .as_os_str()
                .as_encoded_bytes()
                .iter()
                .any(|c| matches!(c, b',' | b';' | b'\r' | b'\n'))
            || !(1_500_000..=35_000_000).contains(&frequency_hz)
        {
            return Err(io::Error::other(
                "explicit absolute evidence directory and HF sampler frequency required",
            ));
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let meta = directory.metadata()?;
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err(io::Error::other(
                "measurement evidence directory must be owner-private (0700)",
            ));
        }
        Ok(Arc::new(Self {
            directory,
            display_path: path.to_owned(),
            frequency_hz,
            slot: Mutex::new(Slot {
                id: 0,
                phase: PhaseRecord::Idle,
                records: None,
                export_attempt: 0,
            }),
        }))
    }
    pub(crate) fn reserve(
        self: &Arc<Self>,
        client: u64,
        operator: Arc<OperatorIngress>,
        cancel: Cancel,
    ) -> Result<Ticket, Fault> {
        let mut slot = self.slot.lock_unpoisoned();
        if !matches!(
            slot.phase,
            PhaseRecord::Idle | PhaseRecord::Exported | PhaseRecord::Rejected
        ) {
            return Err(Fault::CleanupPending);
        }
        slot.id = slot.id.checked_add(1).ok_or(Fault::InvalidConfig)?;
        slot.phase = PhaseRecord::Queued;
        slot.records = None;
        slot.export_attempt = 0;
        Ok(Ticket {
            host: self.clone(),
            id: slot.id,
            client,
            operator,
            cancel,
        })
    }
    /// Runs on the control thread, never inside the native/TX scheduling loop.
    /// Paths are host-selected; a client supplies only a checked numeric ID.
    pub(crate) fn export(&self, id: u64) -> Result<String, String> {
        let (records, attempt) = {
            let mut slot = self.slot.lock_unpoisoned();
            if slot.id != id || slot.phase != PhaseRecord::Complete {
                return Err("CleanupPendingOrUnknownCapture".into());
            }
            let attempt = slot
                .export_attempt
                .checked_add(1)
                .ok_or("ExportSequenceExhausted")?;
            let records = slot.records.take().ok_or("MissingRecorder")?;
            slot.export_attempt = attempt;
            slot.phase = PhaseRecord::Exporting;
            (records, attempt)
        };
        let name = format!("measurement-{id}-{attempt}.json");
        let temporary = format!("{name}.partial");
        let result = (|| -> io::Result<()> {
            let c_name = std::ffi::CString::new(temporary.as_str())?;
            let fd = unsafe {
                libc::openat(
                    self.directory.as_raw_fd(),
                    c_name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut file = unsafe { File::from_raw_fd(fd) };
            // External coupling has NOT been calibrated. The recorder reports
            // commanded ADC attenuation separately; neither is LP700 RF power.
            let json = format!("{{\"format\":\"saturn-live-measurement-envelope-v1\",\"id\":{id},\"frequencyHz\":{},\"feedbackInput\":\"rx-bypass\",\"externalCouplingDb\":null,\"lp700Measurement\":null,\"rfImdQualified\":false,\"measurement\":{}}}\n", self.frequency_hz, records.export_json());
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
            let target = std::ffi::CString::new(name.as_str())?;
            // Never replace an existing export, even if a process restarted
            // with the same counter. Keep partial files on every failed attempt.
            let renamed = unsafe {
                libc::renameat2(
                    self.directory.as_raw_fd(),
                    c_name.as_ptr(),
                    self.directory.as_raw_fd(),
                    target.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            };
            if renamed != 0 {
                return Err(io::Error::last_os_error());
            }
            self.directory.sync_all()
        })();
        let mut slot = self.slot.lock_unpoisoned();
        match result {
            Ok(()) => {
                slot.phase = PhaseRecord::Exported;
                Ok(self.display_path.join(name).display().to_string())
            }
            Err(error) => {
                slot.records = Some(records); // failed export cannot discard evidence
                slot.phase = PhaseRecord::Complete;
                Err(format!("ExportIo:{:?}", error.kind()))
            }
        }
    }
}
pub(crate) struct Ticket {
    host: Arc<Host>,
    pub(crate) id: u64,
    client: u64,
    operator: Arc<OperatorIngress>,
    cancel: Cancel,
}
impl Ticket {
    fn reply(&self, status: &str) -> bool {
        self.operator.settings_reply(
            self.client,
            format!("saturn_ps_measurement:0,{},{};", self.id, status),
            || self.cancel.release(),
        )
    }
    pub(crate) fn queued(&self) -> bool {
        self.reply("queued")
    }
    pub(crate) fn running(&self) {
        {
            let mut slot = self.host.slot.lock_unpoisoned();
            if slot.id != self.id || slot.phase != PhaseRecord::Queued {
                return;
            }
            slot.phase = PhaseRecord::Running;
        }
        self.reply("admitted"); // owner acceptance, NOT proof of physical RF
    }
    pub(crate) fn reject(&self, fault: Fault) {
        {
            let mut slot = self.host.slot.lock_unpoisoned();
            if slot.id != self.id || slot.phase != PhaseRecord::Queued {
                return;
            }
            slot.phase = PhaseRecord::Rejected;
        }
        self.reply(&format!("rejected,{fault:?}"));
    }
    /// A contended control mailbox cannot postpone release or DSP progress.
    pub(crate) fn service<N: CooperativeNative, A: TxAuthority>(
        &self,
        life: &mut TxLifecycle<N, A>,
    ) -> bool {
        let status = life.status();
        if status.phase == Phase::Blocked {
            if let Ok(mut slot) = self.host.slot.try_lock() {
                if slot.id == self.id && slot.phase == PhaseRecord::Running {
                    slot.phase = PhaseRecord::Blocked;
                    drop(slot);
                    self.reply("blocked,CleanupIncomplete");
                }
            }
            return false; // no extraction, no export, no re-arm
        }
        if status.phase != Phase::ReadyOff || status.cleanup_completed_ns.is_none() {
            return false;
        }
        let Ok(mut slot) = self.host.slot.try_lock() else {
            return false;
        };
        if slot.id != self.id || slot.phase != PhaseRecord::Running {
            return false;
        }
        let Ok(Some(records)) = life.take_measurement_records() else {
            return false;
        };
        let count = records.records().len();
        slot.records = Some(records);
        slot.phase = PhaseRecord::Complete;
        drop(slot);
        self.reply(&format!(
            "complete,{count},{}",
            status
                .primary_fault
                .or(status.authority_fault)
                .or(status.cleanup_fault)
                .map_or_else(|| "none".into(), |f| format!("{f:?}"))
        ));
        true
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.reject(Fault::GenerationMismatch);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use saturn_bridge::puresignal::measurement::CaptureContext;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    pub(crate) fn directory() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "saturn-ps-measurement-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        path
    }
    fn finished_recorder() -> MeasurementRecorder {
        let mut r = MeasurementRecorder::new(
            1,
            CaptureContext {
                source_pin: "synthetic-test-not-RF".into(),
                native_identity: "fixture".into(),
                feedback_attenuation_db: Some(31),
                commanded_drive: Some(1),
                physical_adc: saturn_bridge::puresignal::OverloadSource::Adc1,
            },
        )
        .unwrap();
        r.finish(Some(Fault::Stale), Some(50));
        r
    }
    fn complete(host: &Host, id: u64) {
        let mut slot = host.slot.lock_unpoisoned();
        slot.id = id;
        slot.phase = PhaseRecord::Complete;
        slot.records = Some(finished_recorder());
    }
    #[test]
    fn evidence_directory_requires_private_absolute_nonsymlink_path_and_sampler_band() {
        let path = directory();
        assert!(Host::open(Path::new("relative"), 7_200_000).is_err());
        assert!(Host::open(&path, 50_100_000).is_err());
        assert!(Host::open(&path, 27_385_000).is_ok());
        let link = path.with_extension("symlink");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(Host::open(&link, 7_200_000).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Host::open(&path, 7_200_000).is_err());
    }
    #[test]
    fn export_requires_matching_capture_and_completed_cleanup() {
        let path = directory();
        let h = Host::open(&path, 7_200_000).unwrap();
        for phase in [
            PhaseRecord::Idle,
            PhaseRecord::Queued,
            PhaseRecord::Running,
            PhaseRecord::Blocked,
            PhaseRecord::Exporting,
        ] {
            {
                let mut s = h.slot.lock_unpoisoned();
                s.id = 1;
                s.phase = phase;
            }
            assert!(h.export(1).is_err());
            assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
        }
        complete(&h, 1);
        assert!(h.export(2).is_err());
        let filename = h.export(1).unwrap();
        let json = std::fs::read_to_string(&filename).unwrap();
        assert!(json.contains("\"externalCouplingDb\":null"));
        assert!(json.contains("\"recordingComplete\":false"));
        assert!(json.contains("\"terminalFault\":\"Stale\""));
        assert_eq!(std::fs::metadata(filename).unwrap().mode() & 0o777, 0o600);
        assert!(h.export(1).is_err());
    }
    #[test]
    fn failed_export_preserves_records_and_never_overwrites_previous_evidence() {
        let path = directory();
        let h = Host::open(&path, 7_200_000).unwrap();
        complete(&h, 1);
        std::fs::write(path.join("measurement-1-1.json"), b"original evidence").unwrap();
        assert!(h.export(1).is_err());
        assert_eq!(
            std::fs::read(path.join("measurement-1-1.json")).unwrap(),
            b"original evidence"
        );
        assert!(path.join("measurement-1-1.json.partial").exists());
        assert!(h.slot.lock_unpoisoned().records.is_some());
        assert!(h.export(1).unwrap().ends_with("measurement-1-2.json"));
    }
}
