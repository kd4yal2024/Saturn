//! Shared-owner observation only. Original samples and generation identity are
//! copied at existing boundaries; this module has no radio/native/arm handle.
use crate::{radio_model::DemodMode, tx_thread::feedback_owner::Scope};
use saturn_bridge::puresignal::{Backend, BlockMeta, CounterBracket, PairBlock};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const CAPACITY: usize = 8;
const BUDGET: usize = 256;
const VALUES: usize = 8192;
const ARRIVALS: usize = 1024;
const BYTES: usize = 6144;
#[derive(Clone, Copy, Debug)]
pub(crate) enum Kind {
    Pcm,
    Wdsp,
    H2c,
    Pairs,
    Key,
    Stop,
    Clean,
    Stale,
}
impl Kind {
    fn index(self) -> usize {
        self as usize
    }
}
#[derive(Clone, Copy)]
struct Header {
    kind: Kind,
    scope: Scope,
    backend: Backend,
    watts: u8,
    mode: DemodMode,
    source_oldest: Option<u64>,
    submitted: Option<u64>,
    completed: Option<u64>,
    recorded: u64,
    keyed: Option<bool>,
    requested: Option<usize>,
    written: Option<usize>,
    first_pair: Option<u64>,
    arrivals_recorded: bool,
    feedback_meta: Option<BlockMeta>,
}
struct Record {
    header: Option<Header>,
    values: Vec<f64>,
    raw: Vec<u8>,
    arrivals: Vec<u64>,
}
struct Slots {
    free: Vec<Record>,
    ready: VecDeque<Record>,
    budget: usize,
    next: [u64; 8],
    generation: Option<(u128, u64, u64)>,
}
struct Inner {
    slots: Mutex<Slots>,
    origin: Instant,
    stop: AtomicBool,
    drops: AtomicU64,
    exhausted: AtomicU64,
    errors: AtomicU64,
}
#[derive(Clone)]
pub(crate) struct Producer(Arc<Inner>);
#[derive(Clone)]
pub(crate) struct Submission {
    pub(crate) producer: Producer,
    scope: Scope,
    backend: Backend,
    watts: u8,
    mode: DemodMode,
    source_oldest: Option<u64>,
}
pub(crate) struct Worker {
    producer: Producer,
    thread: Option<JoinHandle<()>>,
}
impl Producer {
    pub(crate) fn now(&self) -> u64 {
        self.0
            .origin
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX - 1)) as u64
    }
    pub(crate) fn submission(
        &self,
        scope: Scope,
        backend: Backend,
        watts: u8,
        mode: DemodMode,
        source_oldest: Option<u64>,
    ) -> Submission {
        Submission {
            producer: self.clone(),
            scope,
            backend,
            watts,
            mode,
            source_oldest,
        }
    }
    fn capture(
        &self,
        h: Header,
        values: impl ExactSizeIterator<Item = f64>,
        raw: &[u8],
        arrivals: &[u64],
    ) {
        if values.len() > VALUES || raw.len() > BYTES || arrivals.len() > ARRIVALS {
            self.0.drops.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let Ok(mut q) = self.0.slots.try_lock() else {
            self.0.drops.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if self.0.stop.load(Ordering::Acquire) {
            self.0.drops.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let generation = (
            h.scope.fence.boot,
            h.scope.output.generation,
            h.scope.output.revision,
        );
        if q.generation != Some(generation) {
            q.next = [0; 8];
            q.generation = Some(generation);
        }
        // Rate limit samples, not lifecycle receipts. One bounded record per
        // point per 100 ms. These are retained windows, not a continuous trace.
        if matches!(h.kind, Kind::Pcm | Kind::Wdsp | Kind::H2c | Kind::Pairs)
            && h.recorded < q.next[h.kind.index()]
        {
            return;
        }
        if q.budget == 0 {
            self.0.exhausted.fetch_add(1, Ordering::Relaxed);
            return;
        }
        // Leave a small per-process reserve for fault/stop/cleanup evidence.
        // This does not enlarge the fixed budget or reset it on a later arm.
        if q.budget <= 16 && matches!(h.kind, Kind::Pcm | Kind::Wdsp | Kind::H2c | Kind::Pairs) {
            self.0.exhausted.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let Some(mut r) = q.free.pop() else {
            self.0.drops.fetch_add(1, Ordering::Relaxed);
            return;
        };
        q.next[h.kind.index()] = h.recorded.saturating_add(100_000_000);
        r.header = Some(h);
        r.values.clear();
        r.values.extend(values);
        r.raw.clear();
        r.raw.extend_from_slice(raw);
        r.arrivals.clear();
        r.arrivals.extend_from_slice(arrivals);
        q.budget -= 1;
        q.ready.push_back(r);
    }
    pub(crate) fn pairs(&self, scope: Scope, block: &PairBlock) {
        if block.pairs == 0 || block.pairs > ARRIVALS || block.meta.fence != scope.fence {
            self.0.drops.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let h = Header {
            kind: Kind::Pairs,
            scope,
            backend: block.meta.backend,
            watts: 0,
            mode: DemodMode::Unknown,
            source_oldest: Some(block.meta.observed_ns),
            submitted: None,
            completed: None,
            recorded: self.now(),
            keyed: None,
            requested: None,
            written: None,
            first_pair: Some(block.meta.interval.first),
            arrivals_recorded: block.arrival_times_recorded,
            feedback_meta: Some(block.meta),
        };
        self.capture(
            h,
            (0..block.pairs * 4).map(|i| {
                if i % 4 < 2 {
                    block.tx[i / 4 * 2 + i % 4]
                } else {
                    block.rf[i / 4 * 2 + i % 4 - 2]
                }
            }),
            &[],
            if block.arrival_times_recorded {
                &block.arrival_ns[..block.pairs]
            } else {
                &[]
            },
        );
    }
}
impl Submission {
    fn header(&self, kind: Kind, keyed: Option<bool>) -> Header {
        Header {
            kind,
            scope: self.scope,
            backend: self.backend,
            watts: self.watts,
            mode: self.mode,
            source_oldest: self.source_oldest,
            submitted: None,
            completed: None,
            recorded: self.producer.now(),
            keyed,
            requested: None,
            written: None,
            first_pair: None,
            arrivals_recorded: false,
            feedback_meta: None,
        }
    }
    pub(crate) fn pcm(&self, samples: &[f32]) {
        self.producer.capture(
            self.header(Kind::Pcm, None),
            samples.iter().copied().map(f64::from),
            &[],
            &[],
        );
    }
    pub(crate) fn wdsp(&self, samples: &[f32]) {
        self.producer.capture(
            self.header(Kind::Wdsp, None),
            samples.iter().copied().map(f64::from),
            &[],
            &[],
        );
    }
    pub(crate) fn event(&self, kind: Kind, keyed: Option<bool>) {
        self.producer
            .capture(self.header(kind, keyed), std::iter::empty(), &[], &[]);
    }
    /// Eight exact u64 fields encoded as low/high u32 doubles. No f64 rounding
    /// of nanosecond timestamps, formatting, allocation or I/O on the owner.
    pub(crate) fn stale(&self, fields: [u64; 8]) {
        self.producer.capture(
            self.header(Kind::Stale, None),
            (0..16).map(|i| f64::from((fields[i / 2] >> (32 * (i % 2))) as u32)),
            &[],
            &[],
        );
    }
    pub(crate) fn key_readback(&self, words: &[u32]) {
        self.producer.capture(
            self.header(Kind::Key, Some(true)),
            words.iter().copied().map(f64::from),
            &[],
            &[],
        );
    }
    pub(crate) fn completed(
        &self,
        submitted: u64,
        bytes: &[u8],
        written: Option<usize>,
        keyed: bool,
    ) {
        let mut h = self.header(Kind::H2c, Some(keyed));
        h.submitted = Some(submitted);
        h.completed = Some(h.recorded);
        h.requested = Some(bytes.len());
        h.written = written;
        self.producer.capture(h, std::iter::empty(), bytes, &[]);
    }
}
fn optional<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map_or("null".into(), |v| v.to_string())
}
fn bracket(b: Option<CounterBracket>) -> String {
    b.map_or("null".into(),|b|format!(concat!("{{\"source\":\"{:?}\",\"boot\":\"{}\",\"generation\":\"{}\",\"config\":\"{}\",\"txEpoch\":\"{}\",",
        "\"first\":\"{}\",\"end\":\"{}\",\"observedNs\":{},\"counterGeneration\":\"{}\",\"endCounterGeneration\":\"{}\",\"startCount\":\"{}\",\"endCount\":\"{}\",",
        "\"widthBits\":{},\"saturated\":{},\"overflow\":{},\"complete\":{},\"valid\":{}}}"),
        b.source,b.interval.fence.boot,b.interval.fence.generation,b.interval.fence.config,b.interval.fence.tx_epoch,
        b.interval.first,b.interval.end,b.observed_ns,b.counter_generation,b.end_counter_generation,b.start_count,b.end_count,
        optional(b.width_bits),optional(b.saturated),optional(b.overflow),b.complete,b.valid))
}
fn export(directory: &Path, id: u64, r: &Record) -> io::Result<()> {
    let h = r.header.unwrap();
    let f = h.scope.fence;
    let o = h.scope.output;
    let base = format!("capture-{id:04}");
    let mut samples = io::BufWriter::new(
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.join(format!("{base}.f64le")))?,
    );
    let mut values_hash = Sha256::new();
    for s in &r.values {
        let bytes = s.to_le_bytes();
        samples.write_all(&bytes)?;
        values_hash.update(bytes);
    }
    samples.flush()?;
    let values_hash = format!("{:x}", values_hash.finalize());
    let packed_hash = format!("{:x}", Sha256::digest(&r.raw));
    let mut arrival_hash = Sha256::new();
    for t in &r.arrivals {
        arrival_hash.update(t.to_le_bytes());
    }
    let arrival_hash = format!("{:x}", arrival_hash.finalize());
    if !r.raw.is_empty() {
        let mut out = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.join(format!("{base}.packed")))?;
        out.write_all(&r.raw)?;
    }
    if !r.arrivals.is_empty() {
        let mut out = io::BufWriter::new(
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(directory.join(format!("{base}.arrival-u64le")))?,
        );
        for t in &r.arrivals {
            out.write_all(&t.to_le_bytes())?;
        }
        out.flush()?;
    }
    let text=format!(concat!("{{\"format\":\"saturn-shared-signal-capture-v1\",\"kind\":\"{:?}\",\"backend\":\"{:?}\",",
        "\"boot\":\"{}\",\"generation\":\"{}\",\"config\":\"{}\",\"txEpoch\":\"{}\",",
        "\"outputGeneration\":\"{}\",\"outputRevision\":\"{}\",\"route\":\"{}\",\"frequencyHz\":{},\"attenuationCommandDb\":{},",
        "\"commandedWatts\":{},\"mode\":\"{:?}\",\"stimulus\":\"{}\",\"sourceOldestNs\":{},\"submittedNs\":{},\"completedNs\":{},\"recordedNs\":{},",
        "\"keyedSnapshot\":{},\"requestedBytes\":{},\"completedBytes\":{},\"values\":{},\"packedBytes\":{},",
        "\"firstReturnedPair\":{},\"arrivalTimesRecorded\":{},\"arrivalCount\":{},\"overload\":\"unknown\",\"overloadEvidence\":[{},{},{}],",
        "\"returnedIntervalEnd\":{},\"sequence\":{},\"valuesSha256\":\"{}\",\"packedSha256\":\"{}\",\"arrivalSha256\":\"{}\",\"rfQualified\":false}}\n"),
        h.kind,h.backend,f.boot,f.generation,f.config,f.tx_epoch,o.generation,o.revision,h.scope.route,h.scope.frequency_hz,h.scope.attenuation_db,
        if matches!(h.kind,Kind::Pairs) {"null".into()} else {h.watts.to_string()},h.mode,h.scope.stimulus.label(),optional(h.source_oldest),optional(h.submitted),optional(h.completed),h.recorded,
        optional(h.keyed),optional(h.requested),optional(h.written),r.values.len(),r.raw.len(),optional(h.first_pair),h.arrivals_recorded,r.arrivals.len(),
        bracket(h.feedback_meta.and_then(|m|m.overload[0])),bracket(h.feedback_meta.and_then(|m|m.overload[1])),bracket(h.feedback_meta.and_then(|m|m.overload[2])),
        optional(h.feedback_meta.map(|m|m.interval.end)),optional(h.feedback_meta.and_then(|m|m.sequence)),values_hash,packed_hash,arrival_hash);
    let mut out = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(directory.join(format!("{base}.json")))?;
    out.write_all(text.as_bytes())
}
impl Worker {
    pub(crate) fn from_env(origin: Instant) -> io::Result<Option<Self>> {
        let Some(base) = std::env::var_os("SATURN_TX_DIAGNOSTIC_DIRECTORY") else {
            return Ok(None);
        };
        Self::start_process(Path::new(&base), origin).map(Some)
    }
    fn start_process(base: &Path, origin: Instant) -> io::Result<Self> {
        let path = crate::capture_directory::fresh_path(base)?;
        let worker = Self::start(&path, origin)?;
        eprintln!(
            "saturn-bridge: shared diagnostic archive={}",
            path.display()
        );
        Ok(worker)
    }
    pub(crate) fn start(directory: &Path, origin: Instant) -> io::Result<Self> {
        #[cfg(test)]
        return Self::start_inner(directory, origin, None);
        #[cfg(not(test))]
        return Self::start_inner(directory, origin);
    }
    fn start_inner(
        directory: &Path,
        origin: Instant,
        #[cfg(test)] gate: Option<Arc<std::sync::Barrier>>,
    ) -> io::Result<Self> {
        if !directory.is_absolute() {
            return Err(io::Error::other(
                "NEW absolute shared capture directory required",
            ));
        }
        fs::create_dir(directory)?;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        let free = (0..CAPACITY)
            .map(|_| Record {
                header: None,
                values: Vec::with_capacity(VALUES),
                raw: Vec::with_capacity(BYTES),
                arrivals: Vec::with_capacity(ARRIVALS),
            })
            .collect();
        let producer = Producer(Arc::new(Inner {
            origin,
            slots: Mutex::new(Slots {
                free,
                ready: VecDeque::with_capacity(CAPACITY),
                budget: BUDGET,
                next: [0; 8],
                generation: None,
            }),
            stop: AtomicBool::new(false),
            drops: AtomicU64::new(0),
            exhausted: AtomicU64::new(0),
            errors: AtomicU64::new(0),
        }));
        let context=format!(concat!("{{\"format\":\"saturn-shared-signal-context-v1\",\"sourcePin\":\"{}\",\"wdspArchiveSha256\":\"{}\",",
            "\"timeDomain\":\"shared owner monotonic origin; Bridge ingress/host processing, not FPGA or RF time\",",
            "\"scopeMeaning\":\"expected keyed acquisition fence; output generation/revision identify the sealed arm across key transition\",",
            "\"pcm\":\"48kHz mono f64le promoted exactly from f32\",\"wdsp\":\"192kHz interleaved I,Q f64le promoted exactly from f32\",",
            "\"h2c\":\"exact submitted signed24 big-endian Q,I bytes retained AFTER write returns; pack divisor 8388607\",",
            "\"pairs\":\"192kHz refI,refQ,rfI,rfQ f64le; signed24 /8388608; original per-pair ingress u64le when supplied\",",
            "\"sampling\":\"retained windows <=10 per point/sec, not sample-aligned or continuous; P2 has no H2C boundary\",",
            "\"staleSnapshot\":\"8 u64 as low/high u32 doubles: origin,now,armedAt,lastArrival,micOldest,txLimit,sourceLimit,micLimit; u64MAX means unknown timestamp; origins 0unknown 1duration 2sourceStall 3retainedMic 4ingress 5output 6lifecycle\",\"lifecycleReservedRecords\":16,",
            "\"budget\":{},\"overloadQualification\":\"unknown; consult original measurement evidence\",\"rfQualified\":false}}\n"),
            env!("SATURN_BRIDGE_SOURCE_PIN"),env!("SATURN_BRIDGE_WDSP_ARCHIVE_SHA256"),BUDGET);
        fs::write(directory.join("context.json"), context)?;
        let path = directory.to_path_buf();
        let p = producer.clone();
        let worker=thread::Builder::new().name("saturn-shared-diag".into()).spawn(move || {
            #[cfg(test)]
            if let Some(gate) = gate { gate.wait(); }
            let mut count=0;
            loop {
                let (record,done)={let mut q=p.0.slots.lock().unwrap_or_else(|e|e.into_inner());let r=q.ready.pop_front();let done=r.is_none()&&p.0.stop.load(Ordering::Acquire);(r,done)};
                if let Some(r)=record {
                    if export(&path,count,&r).is_err() {p.0.errors.fetch_add(1,Ordering::Relaxed);}count+=1;
                    p.0.slots.lock().unwrap_or_else(|e|e.into_inner()).free.push(r);continue;
                }
                if done {break;}
                thread::sleep(Duration::from_millis(2));
            }
            let _=fs::write(path.join("capture-summary.json"),format!("{{\"recordsConsumed\":{count},\"diagnosticDrops\":{},\"budgetExhausted\":{},\"exportErrors\":{},\"drained\":true,\"rfQualified\":false}}\n",
                p.0.drops.load(Ordering::Relaxed),p.0.exhausted.load(Ordering::Relaxed),p.0.errors.load(Ordering::Relaxed)));
        })?;
        Ok(Self {
            producer,
            thread: Some(worker),
        })
    }
    pub(crate) fn producer(&self) -> Producer {
        self.producer.clone()
    }
    #[cfg(test)]
    pub(crate) fn finish(mut self) {
        self.producer.0.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.producer.0.stop.store(true, Ordering::Release);
        let _detached = self.thread.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_restart_records_to_fresh_child_and_keeps_completed_capture() {
        let base = crate::tx_thread::ps_measurement::tests::directory().join("restart-archive");
        Worker::start_process(&base, Instant::now())
            .unwrap()
            .finish();
        let before = fs::read(base.join("capture-summary.json")).unwrap();
        Worker::start_process(&base, Instant::now())
            .unwrap()
            .finish();
        assert_eq!(fs::read(base.join("capture-summary.json")).unwrap(), before);
        let children: Vec<_> = fs::read_dir(&base)
            .unwrap()
            .flatten()
            .filter(|e| e.file_type().unwrap().is_dir())
            .collect();
        assert_eq!(children.len(), 1);
        assert!(children[0].path().join("context.json").is_file());
        assert!(children[0].path().join("capture-summary.json").is_file());
    }
    fn scope() -> Scope {
        Scope {
            fence: saturn_bridge::puresignal::Fence {
                boot: (1_u128 << 90) + 7,
                generation: 23,
                config: 31,
                tx_epoch: 11,
            },
            route: 3,
            frequency_hz: 7_200_000,
            attenuation_db: 0,
            output: crate::tx_owner_stop::OutputStamp {
                generation: 42,
                revision: 19,
            },
            stimulus: crate::tx_thread::feedback_owner::Stimulus::TwoTone,
        }
    }
    fn worker() -> (Worker, std::path::PathBuf) {
        let path = crate::tx_thread::ps_measurement::tests::directory().join("signal-unit");
        (Worker::start(&path, Instant::now()).unwrap(), path)
    }
    #[test]
    fn shared_capture_exact_bytes_original_age_and_wide_identity_survive_export() {
        // This test checks exact export, not deliberate try_lock drops. Hold
        // the consumer until all producer submissions have been accepted.
        let path = crate::tx_thread::ps_measurement::tests::directory().join("signal-unit");
        let gate = Arc::new(std::sync::Barrier::new(2));
        let w = Worker::start_inner(&path, Instant::now(), Some(gate.clone())).unwrap();
        let p = w.producer();
        let c = p.submission(scope(), Backend::DirectXdma, 5, DemodMode::Usb, Some(7));
        let native = [-0.25_f32, 0.5, 0.125, -0.75];
        c.wdsp(&native);
        let packed = [0x7f, 0xff, 0xff, 0x80, 0, 0];
        c.completed(8, &packed, Some(6), true);
        let mut pairs = PairBlock::default();
        pairs.pairs = 1;
        pairs.meta.fence = scope().fence;
        pairs.meta.backend = Backend::DirectXdma;
        pairs.meta.interval = saturn_bridge::puresignal::Interval {
            fence: scope().fence,
            first: 91,
            end: 92,
        };
        pairs.meta.observed_ns = 7;
        pairs.arrival_times_recorded = true;
        pairs.arrival_ns[0] = 7;
        pairs.tx[..2].copy_from_slice(&[0.25, -0.5]);
        pairs.rf[..2].copy_from_slice(&[0.125, -0.25]);
        p.pairs(scope(), &pairs);
        gate.wait();
        w.finish();
        let native_bytes: Vec<_> = native
            .into_iter()
            .flat_map(|v| f64::from(v).to_le_bytes())
            .collect();
        assert_eq!(
            fs::read(path.join("capture-0000.f64le")).unwrap(),
            native_bytes
        );
        assert_eq!(fs::read(path.join("capture-0001.packed")).unwrap(), packed);
        let meta = fs::read_to_string(path.join("capture-0002.json")).unwrap();
        assert!(meta.contains(&format!("\"boot\":\"{}\"", scope().fence.boot)));
        assert!(meta.contains("\"sourceOldestNs\":7"));
        assert!(meta.contains("\"outputGeneration\":\"42\""));
        assert_eq!(
            fs::read(path.join("capture-0002.arrival-u64le")).unwrap(),
            7_u64.to_le_bytes()
        );
        assert!(meta.contains("\"overloadEvidence\":[null,null,null]"));
        assert!(fs::read_to_string(path.join("capture-summary.json"))
            .unwrap()
            .contains("\"exportErrors\":0"));
    }
    #[test]
    fn shared_capture_contention_and_wrong_scope_drop_without_blocking_or_retimestamps() {
        let (w, _) = worker();
        let p = w.producer();
        let c = p.submission(scope(), Backend::DirectXdma, 5, DemodMode::Usb, Some(1));
        let held = p.0.slots.lock().unwrap();
        c.wdsp(&[0.1; 20]);
        assert_eq!(p.0.drops.load(Ordering::Relaxed), 1);
        drop(held);
        let mut pairs = PairBlock::default();
        pairs.pairs = 1;
        p.pairs(scope(), &pairs);
        assert_eq!(p.0.drops.load(Ordering::Relaxed), 2);
        w.finish();
        c.wdsp(&[0.1; 20]);
        assert_eq!(p.0.drops.load(Ordering::Relaxed), 3);
    }
    #[test]
    fn shared_capture_failed_and_short_write_receipts_never_become_full_writes() {
        let (w, path) = worker();
        let p = w.producer();
        let c = p.submission(scope(), Backend::DirectXdma, 5, DemodMode::Usb, Some(9));
        c.completed(10, &[0; 6], Some(3), true);
        w.finish();
        let m = fs::read_to_string(path.join("capture-0000.json")).unwrap();
        assert!(m.contains("\"requestedBytes\":6,\"completedBytes\":3"));
        assert!(m.contains("\"sourceOldestNs\":9"));
    }
    #[test]
    fn shared_capture_reserved_fault_evidence_preserves_exact_nanosecond_words() {
        let (w, path) = worker();
        let p = w.producer();
        let c = p.submission(scope(), Backend::DirectXdma, 5, DemodMode::Usb, Some(7));
        p.0.slots.lock().unwrap().budget = 16;
        c.pcm(&[0.; 512]);
        assert_eq!(p.0.exhausted.load(Ordering::Relaxed), 1);
        let fields = [
            3,
            (1_u64 << 60) + 17,
            9,
            u64::MAX,
            (1_u64 << 55) + 31,
            45000000000,
            250000000,
            150000000,
        ];
        c.stale(fields);
        c.event(Kind::Stop, Some(false));
        c.event(Kind::Clean, Some(false));
        w.finish();
        let bytes = fs::read(path.join("capture-0000.f64le")).unwrap();
        let words: Vec<_> = bytes
            .chunks_exact(8)
            .map(|v| f64::from_le_bytes(v.try_into().unwrap()) as u64)
            .collect();
        let restored: Vec<_> = words.chunks_exact(2).map(|v| v[0] | v[1] << 32).collect();
        assert_eq!(restored, fields);
        assert!(fs::read_to_string(path.join("capture-0000.json"))
            .unwrap()
            .contains("\"kind\":\"Stale\""));
        assert!(fs::read_to_string(path.join("capture-0002.json"))
            .unwrap()
            .contains("\"kind\":\"Clean\""));
    }
}
