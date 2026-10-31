//! Opt-in Legacy/Direct-XDMA diagnostics. Never owns a device, native channel
//! or TX permission. Producer work is bounded copies/atomics/try_lock only.
#[cfg(feature = "offline-puresignal-owner-adapters")]
pub(crate) mod shared;
use saturn_bridge::signal_diagnostics::{
    self as d, Backend, CaptureTap, Event, Evidence, Meta, Point, TimeDomain,
};
use std::os::unix::fs::PermissionsExt;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
struct Inner {
    domain: TimeDomain,
    tap: CaptureTap,
    origin: Instant,
    session: u128,
    generation: AtomicU64,
    next: [AtomicU64; 6],
    stop: AtomicBool,
    io_errors: AtomicU64,
}
#[derive(Clone)]
pub(crate) struct Producer(Arc<Inner>);
pub(crate) struct Worker {
    producer: Producer,
    thread: Option<JoinHandle<()>>,
}
impl Producer {
    pub(crate) fn now(&self) -> u64 {
        self.0.origin.elapsed().as_nanos().min(u64::MAX as u128 - 1) as u64
    }
    fn meta(&self, point: Point, event: Event, at: u64, keyed: Option<bool>) -> Meta {
        Meta {
            point,
            event,
            backend: Backend::DirectXdma,
            session: self.0.session,
            generation: self.0.generation.load(Ordering::Acquire),
            config: 0,
            tx_epoch: self.0.generation.load(Ordering::Acquire),
            observed_first_ns: Some(at),
            observed_last_ns: Some(at),
            recorded_ns: self.now(),
            time_domain: self.0.domain,
            rate_hz: 192000,
            units_per_full_scale: 1.,
            attenuation_command_db: None,
            continuity: Evidence::Unknown,
            overload: Evidence::Unknown,
            keyed,
            requested_bytes: None,
            completed_bytes: None,
        }
    }
    fn due(&self, point: Point, at: u64) -> bool {
        let next = &self.0.next[point as usize];
        let old = next.load(Ordering::Relaxed);
        at >= old
            && next
                .compare_exchange(
                    old,
                    at.saturating_add(100_000_000),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
    }
    pub(crate) fn begin(&self) {
        self.0.generation.fetch_add(1, Ordering::AcqRel);
        for next in &self.0.next {
            next.store(0, Ordering::Relaxed);
        }
        self.event(Event::OperatorRequest, None);
    }
    pub(crate) fn event(&self, event: Event, keyed: Option<bool>) {
        self.0
            .tap
            .try_capture(self.meta(Point::Timeline, event, self.now(), keyed), &[]);
    }
    pub(crate) fn key_readback(&self, words: &[u32]) {
        self.0.tap.try_capture_iter(
            self.meta(Point::Timeline, Event::KeyAsserted, self.now(), Some(true)),
            words.iter().copied().map(f64::from),
        );
    }
    pub(crate) fn control_completed(&self, frequency_hz: u32, drive_code: u32) {
        self.0.tap.try_capture_iter(
            self.meta(
                Point::Timeline,
                Event::ControlChanged,
                self.now(),
                Some(true),
            ),
            [frequency_hz, drive_code].into_iter().map(f64::from),
        );
    }
    pub(crate) fn wdsp(&self, iq: &[f64]) {
        let at = self.now();
        if self.due(Point::WdspIq, at) {
            self.0
                .tap
                .try_capture(self.meta(Point::WdspIq, Event::Samples, at, None), iq);
        }
    }
    /// The aligned submission buffer cannot change while DirectTxState holds
    /// &mut self across write_at. Retain those exact submitted Q/I int24 values
    /// AFTER return, tagged with the original submission time and byte result.
    pub(crate) fn completed(
        &self,
        submitted_ns: u64,
        bytes: &[u8],
        written: Option<usize>,
        keyed: bool,
    ) {
        if !self.due(Point::BeforeOutputIq, submitted_ns) {
            return;
        }
        let mut meta = self.meta(
            Point::BeforeOutputIq,
            Event::Samples,
            submitted_ns,
            Some(keyed),
        );
        meta.units_per_full_scale = 8_388_607.;
        meta.requested_bytes = Some(bytes.len() as u64);
        meta.completed_bytes = written.map(|v| v as u64);
        if bytes.len().is_multiple_of(6) && bytes.len() / 3 <= 8192 {
            self.0.tap.try_capture_iter(
                meta,
                bytes.chunks_exact(3).enumerate().map(|(n, _)| {
                    // Logical I,Q from physical Q,I. Integer values stay exact;
                    // analysis applies the explicit packing full-scale divisor.
                    let index = if n % 2 == 0 { n + 1 } else { n - 1 };
                    let b = &bytes[index * 3..index * 3 + 3];
                    (i32::from_be_bytes([b[0], b[1], b[2], 0]) >> 8) as f64
                }),
            );
        }
        let mut receipt = self.meta(
            Point::Timeline,
            if written == Some(bytes.len()) {
                Event::OutputCompleted
            } else {
                Event::Rejected
            },
            submitted_ns,
            Some(keyed),
        );
        receipt.requested_bytes = meta.requested_bytes;
        receipt.completed_bytes = meta.completed_bytes;
        self.0.tap.try_capture(receipt, &[]);
    }
}
impl Worker {
    pub(crate) fn from_env() -> Option<Self> {
        let path = std::env::var_os("SATURN_TX_DIAGNOSTIC_DIRECTORY")?;
        if std::env::var("SATURN_BRIDGE_TX_OWNER").unwrap_or_else(|_| "legacy".into()) != "legacy" {
            eprintln!("saturn-bridge: signal capture unavailable outside Legacy owner");
            return None;
        }
        match Self::start(Path::new(&path)) {
            Ok(w) => Some(w),
            Err(e) => {
                eprintln!("saturn-bridge: signal capture unavailable: {e}");
                None
            }
        }
    }
    pub(crate) fn start(path: &Path) -> io::Result<Self> {
        Self::start_with_domain(path, TimeDomain::HostProcessing)
    }
    #[cfg(test)]
    pub(crate) fn start_offline(path: &Path) -> io::Result<Self> {
        Self::start_with_domain(path, TimeDomain::OfflineFixture)
    }
    fn start_with_domain(path: &Path, domain: TimeDomain) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::other(
                "absolute NEW diagnostic directory required",
            ));
        }
        fs::create_dir(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        let session = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let template = Meta {
            point: Point::Timeline,
            event: Event::OperatorRequest,
            backend: Backend::DirectXdma,
            session,
            generation: 0,
            config: 0,
            tx_epoch: 0,
            observed_first_ns: None,
            observed_last_ns: None,
            recorded_ns: 0,
            time_domain: TimeDomain::HostProcessing,
            rate_hz: 192000,
            units_per_full_scale: 1.,
            attenuation_command_db: None,
            continuity: Evidence::Unknown,
            overload: Evidence::Unknown,
            keyed: None,
            requested_bytes: None,
            completed_bytes: None,
        };
        let producer = Producer(Arc::new(Inner {
            domain,
            tap: CaptureTap::new(8, 8192, 256, template).map_err(io::Error::other)?,
            origin: Instant::now(),
            session,
            generation: AtomicU64::new(0),
            next: std::array::from_fn(|_| AtomicU64::new(0)),
            stop: AtomicBool::new(false),
            io_errors: AtomicU64::new(0),
        }));
        let directory = path.to_path_buf();
        let p = producer.clone();
        let handle=thread::Builder::new().name("saturn-tx-diag".into()).spawn(move|| {
            let mut count=0;
            let identity=format!("{{\"format\":\"saturn-tx-capture-context-v1\",\"sourcePin\":\"{}\",\"wdspArchiveSha256\":\"{}\",\"session\":\"{}\",\"generationMeaning\":\"Legacy-owner-arm-sequence-NOT-authorization\",\"configIdentityAvailable\":false,\"sampleTimeIsRfTime\":false,\"captureBudget\":256,\"rawCaptureEnabledByExplicitEnvironment\":true}}\n",
                env!("SATURN_BRIDGE_SOURCE_PIN"),env!("SATURN_BRIDGE_WDSP_ARCHIVE_SHA256"),p.0.session);
            if fs::write(directory.join("context.json"),identity).is_err(){p.0.io_errors.fetch_add(1,Ordering::Relaxed);}
            loop {
                if p.0.tap.consume_one(|record| {
                    let path=directory.join(format!("capture-{count:04}.satdiag"));count+=1;
                    let result=(||->io::Result<()> {
                        let file=OpenOptions::new().create_new(true).write(true).open(path)?;
                        let mut file=std::io::BufWriter::with_capacity(131072,file);
                        d::write_record(record,&mut file)?;file.flush()?;
                        if record.meta.point==Point::BeforeOutputIq {
                            let raw=OpenOptions::new().create_new(true).write(true).open(directory.join(format!("submitted-{:04}.bin",count-1)))?;
                            let mut raw=std::io::BufWriter::with_capacity(65536,raw);
                            for pair in record.samples.chunks_exact(2) {
                                for v in [pair[1],pair[0]] {raw.write_all(&(v as i32).to_be_bytes()[1..])?;}
                            }
                            raw.flush()?;
                        }
                        Ok(())
                    })();
                    if result.is_err(){p.0.io_errors.fetch_add(1,Ordering::Relaxed);}
                }).is_some(){continue;}
                if p.0.stop.load(Ordering::Acquire){break;}
                thread::sleep(Duration::from_millis(2));
            }
            let _=fs::write(directory.join("capture-summary.json"),format!("{{\"recordsConsumed\":{count},\"diagnosticDrops\":{},\"budgetExhausted\":{},\"exportErrors\":{},\"rfOutputVerified\":false}}\n",
                p.0.tap.dropped(),p.0.tap.budget_exhausted(),p.0.io_errors.load(Ordering::Relaxed)));
        })?;
        Ok(Self {
            producer,
            thread: Some(handle),
        })
    }
    pub(crate) fn producer(&self) -> Producer {
        self.producer.clone()
    }
    #[cfg(test)]
    pub(crate) fn finish(mut self) {
        self.producer.0.stop.store(true, Ordering::Release);
        if let Some(h) = self.thread.take() {
            h.join().unwrap();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Never join a potentially stalled filesystem worker on a TX/unkey
        // thread. It owns no RF/device/native resources and exits after its
        // bounded queued work; process exit may leave export incomplete.
        self.producer.0.stop.store(true, Ordering::Release);
        let _detached_diagnostic_only = self.thread.take();
    }
}
