use std::fs;
use std::io;
use std::mem;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config::BridgeConfig;
use crate::radio_model::{RadioModel, TxPhase};
use crate::sync_ext::MutexExt;
use crate::tx_audio::{NullTxAudioSink, TxAudioSink};

pub const SATP_MAGIC: &[u8; 4] = b"SAT1";
pub const SATP_VERSION: u8 = 1;
pub const SATP_PACKET_TYPE_TX_AUDIO: u8 = 1;
pub const SATP_SAMPLE_FORMAT_FLOAT32_LE: u8 = 1;
pub const SATP_SAMPLE_RATE_HZ: u32 = 48_000;
pub const SATP_CHANNELS: u8 = 1;
pub const SATP_FRAMES_PER_PACKET: u16 = 128;
pub const SATP_HEADER_BYTES: usize = 32;
pub const SATP_PAYLOAD_BYTES: usize = SATP_FRAMES_PER_PACKET as usize * mem::size_of::<f32>();
pub const SATP_DATAGRAM_BYTES: usize = SATP_HEADER_BYTES + SATP_PAYLOAD_BYTES;
const SATP_STATUS_PATH: &str = "/run/saturn-bridge/satp-status.json";
const RECEIVE_BATCH_LIMIT: usize = 64;
const RECEIVE_BUFFER_BYTES: usize = 2048;
const REQUESTED_SOCKET_BUFFER_BYTES: i32 = 1024 * 1024;
const PLAYOUT_POLL: Duration = Duration::from_micros(250);
const HEALTHY_LIMIT: Duration = Duration::from_millis(50);
const STATUS_WRITE_PERIOD: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SatpHeader {
    pub packet_type: u8,
    pub channels: u8,
    pub sample_format: u8,
    pub session_id: u32,
    pub stream_id: u32,
    pub sequence: u32,
    pub sample_counter: u64,
    pub frame_count: u16,
    pub flags: u16,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PacketError {
    Length,
    Magic,
    Version,
    PacketType,
    Channels,
    SampleFormat,
    FrameCount,
    Timeline,
    NonFiniteSample,
}

#[derive(Clone, Debug)]
struct SatpPacket {
    header: SatpHeader,
    samples: [f32; SATP_FRAMES_PER_PACKET as usize],
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("validated SATP header"),
    )
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated SATP header"),
    )
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("validated SATP header"),
    )
}

fn parse_packet(bytes: &[u8]) -> Result<SatpPacket, PacketError> {
    if bytes.len() != SATP_DATAGRAM_BYTES {
        return Err(PacketError::Length);
    }
    if &bytes[0..4] != SATP_MAGIC {
        return Err(PacketError::Magic);
    }
    if bytes[4] != SATP_VERSION {
        return Err(PacketError::Version);
    }
    let header = SatpHeader {
        packet_type: bytes[5],
        channels: bytes[6],
        sample_format: bytes[7],
        session_id: read_u32(bytes, 8),
        stream_id: read_u32(bytes, 12),
        sequence: read_u32(bytes, 16),
        sample_counter: read_u64(bytes, 20),
        frame_count: read_u16(bytes, 28),
        flags: read_u16(bytes, 30),
    };
    if header.packet_type != SATP_PACKET_TYPE_TX_AUDIO {
        return Err(PacketError::PacketType);
    }
    if header.channels != SATP_CHANNELS {
        return Err(PacketError::Channels);
    }
    if header.sample_format != SATP_SAMPLE_FORMAT_FLOAT32_LE {
        return Err(PacketError::SampleFormat);
    }
    if header.frame_count != SATP_FRAMES_PER_PACKET {
        return Err(PacketError::FrameCount);
    }
    if !header
        .sample_counter
        .is_multiple_of(SATP_FRAMES_PER_PACKET as u64)
    {
        return Err(PacketError::Timeline);
    }

    let mut samples = [0.0f32; SATP_FRAMES_PER_PACKET as usize];
    for (index, sample) in samples.iter_mut().enumerate() {
        let offset = SATP_HEADER_BYTES + index * 4;
        *sample = f32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("validated SATP payload"),
        );
        if !sample.is_finite() {
            return Err(PacketError::NonFiniteSample);
        }
    }
    Ok(SatpPacket { header, samples })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InsertResult {
    Inserted,
    Duplicate,
    Late,
    Replaced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SequenceResult {
    InOrder,
    Gap,
    OutOfOrder,
}

#[derive(Default)]
struct SequenceTracker {
    expected: Option<u32>,
}

impl SequenceTracker {
    fn reset(&mut self) {
        self.expected = None;
    }

    fn observe(&mut self, sequence: u32) -> SequenceResult {
        let Some(expected) = self.expected else {
            self.expected = Some(sequence.wrapping_add(1));
            return SequenceResult::InOrder;
        };
        let forward = sequence.wrapping_sub(expected);
        if forward == 0 {
            self.expected = Some(expected.wrapping_add(1));
            SequenceResult::InOrder
        } else if forward < (1 << 31) {
            self.expected = Some(sequence.wrapping_add(1));
            SequenceResult::Gap
        } else {
            SequenceResult::OutOfOrder
        }
    }
}

struct PacketRing {
    slots: Vec<Option<SatpPacket>>,
    occupied: usize,
    cursor: Option<u64>,
    started: bool,
    target_packets: usize,
}

#[derive(Default)]
struct PlayoutClock {
    next: Option<Instant>,
}

impl PlayoutClock {
    fn reset(&mut self) {
        self.next = None;
    }

    fn prime_if_ready(&mut self, ring: &mut PacketRing, now: Instant, startup_delay: Duration) {
        if self.next.is_none() && ring.begin_if_ready() {
            // Hold the complete jitter target before starting steady playout.
            // The Windows sender emits four 128-frame packets as one burst;
            // using only one packet period here races the next burst at an
            // empty ring instead of maintaining the configured midpoint.
            self.next = Some(now + startup_delay);
        }
    }

    fn is_due(&self, now: Instant) -> bool {
        self.next.is_some_and(|next| now >= next)
    }

    fn advance(&mut self, packet_period: Duration) {
        if let Some(next) = self.next.as_mut() {
            *next += packet_period;
        }
    }
}

impl PacketRing {
    fn new(target_frames: u32, capacity_frames: u32) -> Self {
        let capacity_packets = capacity_frames.div_ceil(SATP_FRAMES_PER_PACKET as u32) as usize;
        let target_packets = target_frames
            .div_ceil(SATP_FRAMES_PER_PACKET as u32)
            .min(capacity_packets as u32) as usize;
        Self {
            slots: (0..capacity_packets.max(1)).map(|_| None).collect(),
            occupied: 0,
            cursor: None,
            started: false,
            target_packets: target_packets.max(1),
        }
    }

    fn clear(&mut self) {
        self.slots.iter_mut().for_each(|slot| *slot = None);
        self.occupied = 0;
        self.cursor = None;
        self.started = false;
    }

    fn frames(&self) -> u32 {
        (self.occupied * SATP_FRAMES_PER_PACKET as usize) as u32
    }

    fn insert(&mut self, packet: SatpPacket) -> InsertResult {
        if self
            .cursor
            .is_some_and(|cursor| packet.header.sample_counter < cursor)
        {
            return InsertResult::Late;
        }
        let packet_number = packet.header.sample_counter / SATP_FRAMES_PER_PACKET as u64;
        let index = packet_number as usize % self.slots.len();
        match self.slots[index].as_ref() {
            Some(existing) if existing.header.sample_counter == packet.header.sample_counter => {
                return InsertResult::Duplicate;
            }
            Some(_) => {
                self.slots[index] = Some(packet);
                return InsertResult::Replaced;
            }
            None => {}
        }
        self.slots[index] = Some(packet);
        self.occupied += 1;
        InsertResult::Inserted
    }

    fn begin_if_ready(&mut self) -> bool {
        if self.started || self.occupied < self.target_packets {
            return self.started;
        }
        self.cursor = self
            .slots
            .iter()
            .filter_map(|slot| slot.as_ref().map(|packet| packet.header.sample_counter))
            .min();
        self.started = self.cursor.is_some();
        self.started
    }

    fn pop_or_silence(&mut self) -> Option<([f32; SATP_FRAMES_PER_PACKET as usize], bool)> {
        if !self.begin_if_ready() {
            return None;
        }
        let cursor = self.cursor.expect("started ring has cursor");
        let packet_number = cursor / SATP_FRAMES_PER_PACKET as u64;
        let index = packet_number as usize % self.slots.len();
        let packet = self.slots[index].take().filter(|packet| {
            if packet.header.sample_counter == cursor {
                true
            } else {
                self.slots[index] = Some(packet.clone());
                false
            }
        });
        if packet.is_some() {
            self.occupied = self.occupied.saturating_sub(1);
        }
        self.cursor = Some(cursor.wrapping_add(SATP_FRAMES_PER_PACKET as u64));
        Some(match packet {
            Some(packet) => (packet.samples, false),
            None => ([0.0; SATP_FRAMES_PER_PACKET as usize], true),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioHealth {
    Waiting,
    Healthy,
    Degraded,
    Lost,
}

impl AudioHealth {
    fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Lost => "lost",
        }
    }
}

fn classify_audio_health(
    last_packet_at: Option<Instant>,
    now: Instant,
    audio_loss_timeout: Duration,
) -> AudioHealth {
    match last_packet_at {
        None => AudioHealth::Waiting,
        Some(at) => {
            let age = now.saturating_duration_since(at);
            if age < HEALTHY_LIMIT {
                AudioHealth::Healthy
            } else if age <= audio_loss_timeout {
                AudioHealth::Degraded
            } else {
                AudioHealth::Lost
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct SatpStatus {
    pub enabled: bool,
    pub running: bool,
    pub bind: String,
    pub requested_socket_buffer_bytes: i32,
    pub actual_socket_buffer_bytes: i32,
    pub source: Option<String>,
    pub session_id: Option<u32>,
    pub stream_id: Option<u32>,
    pub packets_rx: u64,
    pub frames_rx: u64,
    pub gap_events: u64,
    pub packets_missing: u64,
    pub duplicates: u64,
    pub out_of_order: u64,
    pub late: u64,
    pub invalid: u64,
    pub buffer_overruns: u64,
    pub sessions: u64,
    pub session_changes: u64,
    pub buffer_current: u32,
    pub buffer_min: u32,
    pub buffer_max: u32,
    pub buffer_target: u32,
    pub buffer_capacity: u32,
    pub silence_frames: u64,
    pub frames_consumed: u64,
    pub frames_delivered: u64,
    pub playout_gap_events: u64,
    pub playout_primed: bool,
    pub sink_errors: u64,
    pub effective_sample_rate: f64,
    pub packet_rate: f64,
    pub audio_health: AudioHealth,
    pub audio_loss_events: u64,
    pub last_packet_age_ms: Option<u64>,
    pub tx_authorized: bool,
    pub dekey_requests: u64,
}

impl SatpStatus {
    fn new(config: &BridgeConfig) -> Self {
        Self {
            enabled: config.satp_enabled,
            running: false,
            bind: config.satp_bind_addr.to_string(),
            requested_socket_buffer_bytes: REQUESTED_SOCKET_BUFFER_BYTES,
            actual_socket_buffer_bytes: 0,
            source: None,
            session_id: None,
            stream_id: None,
            packets_rx: 0,
            frames_rx: 0,
            gap_events: 0,
            packets_missing: 0,
            duplicates: 0,
            out_of_order: 0,
            late: 0,
            invalid: 0,
            buffer_overruns: 0,
            sessions: 0,
            session_changes: 0,
            buffer_current: 0,
            buffer_min: u32::MAX,
            buffer_max: 0,
            buffer_target: config.satp_jitter_target_frames,
            buffer_capacity: config.satp_jitter_capacity_frames,
            silence_frames: 0,
            frames_consumed: 0,
            frames_delivered: 0,
            playout_gap_events: 0,
            playout_primed: false,
            sink_errors: 0,
            effective_sample_rate: 0.0,
            packet_rate: 0.0,
            audio_health: AudioHealth::Waiting,
            audio_loss_events: 0,
            last_packet_age_ms: None,
            tx_authorized: false,
            dekey_requests: 0,
        }
    }
}

pub struct SatpRuntime {
    status: Arc<Mutex<SatpStatus>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl SatpRuntime {
    pub fn start(config: &BridgeConfig, radio_model: Arc<Mutex<RadioModel>>) -> io::Result<Self> {
        let status = Arc::new(Mutex::new(SatpStatus::new(config)));
        let stop = Arc::new(AtomicBool::new(false));
        if !config.satp_enabled {
            write_status_file(&status.lock_unpoisoned());
            return Ok(Self {
                status,
                stop,
                worker: None,
            });
        }

        let socket = UdpSocket::bind(config.satp_bind_addr)?;
        socket.set_nonblocking(true)?;
        let actual_socket_buffer_bytes = configure_receive_buffer(&socket)?;
        {
            let mut snapshot = status.lock_unpoisoned();
            snapshot.running = true;
            snapshot.actual_socket_buffer_bytes = actual_socket_buffer_bytes;
            write_status_file(&snapshot);
        }
        println!(
            "saturn-bridge: SATP v1 receiver listening on {} sink=null target={} capacity={} SO_RCVBUF={}",
            config.satp_bind_addr,
            config.satp_jitter_target_frames,
            config.satp_jitter_capacity_frames,
            actual_socket_buffer_bytes,
        );

        let worker_status = status.clone();
        let worker_stop = stop.clone();
        let allowed_source_ip = config.satp_allowed_source_ip;
        let jitter_target_frames = config.satp_jitter_target_frames;
        let jitter_capacity_frames = config.satp_jitter_capacity_frames;
        let audio_loss_timeout = config.satp_audio_loss_timeout;
        let worker = thread::Builder::new()
            .name("saturn-satp".into())
            .spawn(move || {
                run_receiver(
                    socket,
                    allowed_source_ip,
                    jitter_target_frames,
                    jitter_capacity_frames,
                    audio_loss_timeout,
                    radio_model,
                    worker_status,
                    worker_stop,
                );
            })?;
        Ok(Self {
            status,
            stop,
            worker: Some(worker),
        })
    }

    #[allow(dead_code)]
    pub fn snapshot(&self) -> SatpStatus {
        self.status.lock_unpoisoned().clone()
    }
}

impl Drop for SatpRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_receiver(
    socket: UdpSocket,
    allowed_source_ip: Option<IpAddr>,
    jitter_target_frames: u32,
    jitter_capacity_frames: u32,
    audio_loss_timeout: Duration,
    radio_model: Arc<Mutex<RadioModel>>,
    status: Arc<Mutex<SatpStatus>>,
    stop: Arc<AtomicBool>,
) {
    let mut ring = PacketRing::new(jitter_target_frames, jitter_capacity_frames);
    let mut sink = NullTxAudioSink::default();
    let mut receive_buffer = [0u8; RECEIVE_BUFFER_BYTES];
    let packet_period =
        Duration::from_secs_f64(SATP_FRAMES_PER_PACKET as f64 / SATP_SAMPLE_RATE_HZ as f64);
    let startup_delay =
        Duration::from_secs_f64(jitter_target_frames as f64 / SATP_SAMPLE_RATE_HZ as f64);
    let mut playout_clock = PlayoutClock::default();
    let mut last_status_write = Instant::now();
    let mut last_packet_at: Option<Instant> = None;
    let mut active_source: Option<SocketAddr> = None;
    let mut session_id: Option<u32> = None;
    let mut stream_id: Option<u32> = None;
    let mut sequence_tracker = SequenceTracker::default();
    let mut previous_tx_authorized = false;
    let mut audio_loss_latched = false;
    let mut measurement_started = Instant::now();
    let mut measurement_packets_base = 0u64;
    let mut first_sample_counter: Option<u64> = None;
    let mut latest_sample_counter: Option<u64> = None;
    let mut previous_playout_was_silence = false;

    while !stop.load(Ordering::Relaxed) {
        let mut did_work = false;
        for _ in 0..RECEIVE_BATCH_LIMIT {
            let (size, source) = match socket.recv_from(&mut receive_buffer) {
                Ok(value) => value,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    eprintln!("saturn-bridge: SATP receive error: {error}");
                    break;
                }
            };
            did_work = true;
            if allowed_source_ip.is_some_and(|allowed| source.ip() != allowed) {
                status.lock_unpoisoned().invalid += 1;
                continue;
            }
            let packet = match parse_packet(&receive_buffer[..size]) {
                Ok(packet) => packet,
                Err(_) => {
                    status.lock_unpoisoned().invalid += 1;
                    continue;
                }
            };
            if active_source.is_some_and(|active| active.ip() != source.ip())
                && session_id == Some(packet.header.session_id)
            {
                status.lock_unpoisoned().invalid += 1;
                continue;
            }
            if session_id == Some(packet.header.session_id)
                && stream_id.is_some_and(|active| active != packet.header.stream_id)
            {
                status.lock_unpoisoned().invalid += 1;
                continue;
            }
            let now = Instant::now();
            if session_id != Some(packet.header.session_id) {
                let changed = session_id.is_some();
                ring.clear();
                playout_clock.reset();
                sequence_tracker.reset();
                first_sample_counter = None;
                measurement_started = now;
                let _ = sink.flush();
                let mut snapshot = status.lock_unpoisoned();
                measurement_packets_base = snapshot.packets_rx;
                snapshot.sessions = snapshot.sessions.saturating_add(1);
                if changed {
                    snapshot.session_changes = snapshot.session_changes.saturating_add(1);
                }
                snapshot.session_id = Some(packet.header.session_id);
                snapshot.stream_id = Some(packet.header.stream_id);
                session_id = Some(packet.header.session_id);
                stream_id = Some(packet.header.stream_id);
                active_source = Some(source);
                previous_playout_was_silence = false;
            }

            let header = packet.header;
            let insert_result = ring.insert(packet);
            {
                let mut snapshot = status.lock_unpoisoned();
                snapshot.packets_rx = snapshot.packets_rx.saturating_add(1);
                snapshot.frames_rx = snapshot.frames_rx.saturating_add(header.frame_count as u64);
                snapshot.source = Some(source.to_string());
                snapshot.stream_id = Some(header.stream_id);
                match insert_result {
                    InsertResult::Inserted | InsertResult::Replaced => {}
                    InsertResult::Duplicate => snapshot.duplicates += 1,
                    InsertResult::Late => snapshot.late += 1,
                }
                if insert_result == InsertResult::Replaced {
                    snapshot.buffer_overruns = snapshot.buffer_overruns.saturating_add(1);
                }
                if insert_result != InsertResult::Duplicate {
                    match sequence_tracker.observe(header.sequence) {
                        SequenceResult::InOrder => {}
                        SequenceResult::Gap => {
                            snapshot.gap_events = snapshot.gap_events.saturating_add(1)
                        }
                        SequenceResult::OutOfOrder => {
                            snapshot.out_of_order = snapshot.out_of_order.saturating_add(1)
                        }
                    }
                }
            }
            first_sample_counter.get_or_insert(header.sample_counter);
            latest_sample_counter = Some(
                latest_sample_counter.map_or(header.sample_counter, |latest| {
                    latest.max(header.sample_counter)
                }),
            );
            last_packet_at = Some(now);
        }

        let tx_authorized = {
            let model = radio_model.lock_unpoisoned();
            model.desired.tx_phase != TxPhase::Rx || model.desired.tx_enabled
        };
        if tx_authorized && !previous_tx_authorized {
            ring.clear();
            let _ = sink.flush();
            playout_clock.reset();
            previous_playout_was_silence = false;
        }
        previous_tx_authorized = tx_authorized;

        let now = Instant::now();
        let audio_lost =
            last_packet_at.is_some_and(|at| now.saturating_duration_since(at) > audio_loss_timeout);
        if audio_lost && !audio_loss_latched {
            ring.clear();
            let _ = sink.flush();
            playout_clock.reset();
            previous_playout_was_silence = false;
            status.lock_unpoisoned().audio_loss_events += 1;
            audio_loss_latched = true;
        } else if !audio_lost {
            audio_loss_latched = false;
        }
        if !audio_lost {
            playout_clock.prime_if_ready(&mut ring, now, startup_delay);
        }
        let mut playout_steps = 0;
        while !audio_lost && playout_clock.is_due(now) && playout_steps < 8 {
            if let Some((samples, silence)) = ring.pop_or_silence() {
                let mut snapshot = status.lock_unpoisoned();
                snapshot.frames_consumed = snapshot
                    .frames_consumed
                    .saturating_add(SATP_FRAMES_PER_PACKET as u64);
                if silence {
                    if !previous_playout_was_silence {
                        snapshot.playout_gap_events = snapshot.playout_gap_events.saturating_add(1);
                    }
                    snapshot.packets_missing = snapshot.packets_missing.saturating_add(1);
                    snapshot.silence_frames = snapshot
                        .silence_frames
                        .saturating_add(SATP_FRAMES_PER_PACKET as u64);
                }
                previous_playout_was_silence = silence;
                if tx_authorized {
                    match sink.write_frames(&samples) {
                        Ok(()) => {
                            snapshot.frames_delivered = snapshot
                                .frames_delivered
                                .saturating_add(SATP_FRAMES_PER_PACKET as u64)
                        }
                        Err(_) => snapshot.sink_errors = snapshot.sink_errors.saturating_add(1),
                    }
                }
            }
            playout_clock.advance(packet_period);
            playout_steps += 1;
            did_work = true;
        }

        {
            let mut snapshot = status.lock_unpoisoned();
            let current = ring.frames();
            snapshot.buffer_current = current;
            snapshot.buffer_min = snapshot.buffer_min.min(current);
            snapshot.buffer_max = snapshot.buffer_max.max(current);
            snapshot.tx_authorized = tx_authorized;
            snapshot.playout_primed = ring.started;
            snapshot.last_packet_age_ms = last_packet_at
                .map(|at| Instant::now().saturating_duration_since(at).as_millis() as u64);
            snapshot.audio_health =
                classify_audio_health(last_packet_at, Instant::now(), audio_loss_timeout);
            let elapsed = measurement_started.elapsed().as_secs_f64();
            if elapsed > 0.0 {
                snapshot.packet_rate =
                    snapshot.packets_rx.saturating_sub(measurement_packets_base) as f64 / elapsed;
            }
            if let (Some(first), Some(latest)) = (first_sample_counter, latest_sample_counter) {
                if elapsed > 0.0 {
                    snapshot.effective_sample_rate = latest.saturating_sub(first) as f64 / elapsed;
                }
            }
        }

        if last_status_write.elapsed() >= STATUS_WRITE_PERIOD {
            let snapshot = status.lock_unpoisoned().clone();
            write_status_file(&snapshot);
            last_status_write = Instant::now();
        }
        if !did_work {
            thread::sleep(PLAYOUT_POLL);
        }
    }
    {
        let mut snapshot = status.lock_unpoisoned();
        snapshot.running = false;
        write_status_file(&snapshot);
    }
    println!("saturn-bridge: SATP receiver stopped");
}

fn configure_receive_buffer(socket: &UdpSocket) -> io::Result<i32> {
    let fd = std::os::fd::AsRawFd::as_raw_fd(socket);
    let requested = REQUESTED_SOCKET_BUFFER_BYTES;
    // SAFETY: fd is a live UDP socket and both pointers refer to initialized
    // integers with the exact sizes supplied to libc.
    unsafe {
        if libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            (&requested as *const i32).cast(),
            mem::size_of::<i32>() as libc::socklen_t,
        ) != 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut actual = 0i32;
        let mut length = mem::size_of::<i32>() as libc::socklen_t;
        if libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            (&mut actual as *mut i32).cast(),
            &mut length,
        ) != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(actual)
    }
}

fn json_string(value: &str) -> String {
    format!("{value:?}")
}

fn status_json(status: &SatpStatus) -> String {
    let optional_string = |value: &Option<String>| {
        value
            .as_ref()
            .map_or_else(|| "null".into(), |value| json_string(value))
    };
    let optional_number =
        |value: Option<u64>| value.map_or_else(|| "null".into(), |value| value.to_string());
    format!(
        concat!(
            "{{\n",
            "  \"version\": 1,\n",
            "  \"enabled\": {},\n  \"running\": {},\n  \"sink\": \"null\",\n",
            "  \"bind\": {},\n  \"requested_socket_buffer_bytes\": {},\n  \"actual_socket_buffer_bytes\": {},\n",
            "  \"source\": {},\n  \"session_id\": {},\n  \"stream_id\": {},\n",
            "  \"packets_rx\": {},\n  \"frames_rx\": {},\n  \"gap_events\": {},\n  \"packets_missing\": {},\n",
            "  \"duplicates\": {},\n  \"out_of_order\": {},\n  \"late\": {},\n  \"invalid\": {},\n",
            "  \"buffer_overruns\": {},\n",
            "  \"sessions\": {},\n  \"session_changes\": {},\n",
            "  \"effective_sample_rate\": {:.3},\n  \"packet_rate\": {:.3},\n",
            "  \"buffer_current\": {},\n  \"buffer_min\": {},\n  \"buffer_max\": {},\n",
            "  \"buffer_target\": {},\n  \"buffer_capacity\": {},\n",
            "  \"silence_frames\": {},\n  \"frames_consumed\": {},\n  \"frames_delivered\": {},\n  \"playout_gap_events\": {},\n  \"playout_primed\": {},\n  \"sink_errors\": {},\n",
            "  \"audio_health\": {},\n  \"audio_loss_events\": {},\n  \"last_packet_age_ms\": {},\n  \"tx_authorized\": {},\n",
            "  \"dekey_requests\": {},\n  \"updated_unix_ms\": {}\n}}\n"
        ),
        status.enabled,
        status.running,
        json_string(&status.bind),
        status.requested_socket_buffer_bytes,
        status.actual_socket_buffer_bytes,
        optional_string(&status.source),
        optional_number(status.session_id.map(u64::from)),
        optional_number(status.stream_id.map(u64::from)),
        status.packets_rx,
        status.frames_rx,
        status.gap_events,
        status.packets_missing,
        status.duplicates,
        status.out_of_order,
        status.late,
        status.invalid,
        status.buffer_overruns,
        status.sessions,
        status.session_changes,
        status.effective_sample_rate,
        status.packet_rate,
        status.buffer_current,
        if status.buffer_min == u32::MAX { 0 } else { status.buffer_min },
        status.buffer_max,
        status.buffer_target,
        status.buffer_capacity,
        status.silence_frames,
        status.frames_consumed,
        status.frames_delivered,
        status.playout_gap_events,
        status.playout_primed,
        status.sink_errors,
        json_string(status.audio_health.as_str()),
        status.audio_loss_events,
        optional_number(status.last_packet_age_ms),
        status.tx_authorized,
        status.dekey_requests,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
}

fn write_status_file(status: &SatpStatus) {
    let path = PathBuf::from(
        std::env::var_os("SATURN_BRIDGE_SATP_STATUS_PATH")
            .unwrap_or_else(|| SATP_STATUS_PATH.into()),
    );
    if let Err(error) = write_atomic(&path, status_json(status).as_bytes()) {
        if status.enabled {
            eprintln!(
                "saturn-bridge: could not write SATP status {}: {error}",
                path.display()
            );
        }
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&temp, bytes)?;
    fs::rename(temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(sequence: u32, sample_counter: u64, fill: f32) -> Vec<u8> {
        let mut bytes = vec![0u8; SATP_DATAGRAM_BYTES];
        bytes[0..4].copy_from_slice(SATP_MAGIC);
        bytes[4] = SATP_VERSION;
        bytes[5] = SATP_PACKET_TYPE_TX_AUDIO;
        bytes[6] = SATP_CHANNELS;
        bytes[7] = SATP_SAMPLE_FORMAT_FLOAT32_LE;
        bytes[8..12].copy_from_slice(&7u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&0u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&sequence.to_le_bytes());
        bytes[20..28].copy_from_slice(&sample_counter.to_le_bytes());
        bytes[28..30].copy_from_slice(&SATP_FRAMES_PER_PACKET.to_le_bytes());
        for index in 0..SATP_FRAMES_PER_PACKET as usize {
            let offset = SATP_HEADER_BYTES + index * 4;
            bytes[offset..offset + 4].copy_from_slice(&fill.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn parses_exact_v1_contract() {
        let parsed = parse_packet(&packet(10, 256, 0.25)).unwrap();
        assert_eq!(parsed.header.session_id, 7);
        assert_eq!(parsed.header.sequence, 10);
        assert_eq!(parsed.header.sample_counter, 256);
        assert_eq!(parsed.samples, [0.25; 128]);
    }

    #[test]
    fn rejects_incompatible_and_non_finite_packets() {
        let mut bytes = packet(0, 0, 0.0);
        bytes[6] = 2;
        assert_eq!(parse_packet(&bytes).unwrap_err(), PacketError::Channels);
        let mut bytes = packet(0, 0, 0.0);
        bytes[SATP_HEADER_BYTES..SATP_HEADER_BYTES + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(
            parse_packet(&bytes).unwrap_err(),
            PacketError::NonFiniteSample
        );
    }

    #[test]
    fn ring_reorders_and_inserts_exact_silence() {
        let mut ring = PacketRing::new(256, 1024);
        ring.insert(parse_packet(&packet(1, 128, 2.0)).unwrap());
        ring.insert(parse_packet(&packet(0, 0, 1.0)).unwrap());
        let (first, missing) = ring.pop_or_silence().unwrap();
        assert!(!missing);
        assert_eq!(first[0], 1.0);
        let (second, missing) = ring.pop_or_silence().unwrap();
        assert!(!missing);
        assert_eq!(second[0], 2.0);
        let (silence, missing) = ring.pop_or_silence().unwrap();
        assert!(missing);
        assert_eq!(silence, [0.0; 128]);
    }

    #[test]
    fn ring_is_fixed_capacity_and_drops_late_timeline() {
        let mut ring = PacketRing::new(128, 256);
        ring.insert(parse_packet(&packet(0, 0, 0.0)).unwrap());
        ring.pop_or_silence().unwrap();
        assert_eq!(
            ring.insert(parse_packet(&packet(0, 0, 0.0)).unwrap()),
            InsertResult::Late
        );
        assert_eq!(ring.slots.len(), 2);
    }

    #[test]
    fn playout_clock_waits_for_fresh_target_after_reset() {
        let period = Duration::from_millis(3);
        let startup_delay = Duration::from_millis(12);
        let first_epoch = Instant::now();
        let mut clock = PlayoutClock::default();
        let mut ring = PacketRing::new(512, 4096);

        clock.prime_if_ready(&mut ring, first_epoch, startup_delay);
        assert!(!clock.is_due(first_epoch + Duration::from_secs(10)));

        for sequence in 0..4 {
            ring.insert(parse_packet(&packet(sequence, u64::from(sequence) * 128, 0.25)).unwrap());
        }
        clock.prime_if_ready(&mut ring, first_epoch, startup_delay);
        assert!(!clock.is_due(first_epoch + period));
        assert!(clock.is_due(first_epoch + startup_delay));

        ring.clear();
        clock.reset();
        let resumed_at = first_epoch + Duration::from_secs(60);
        assert!(!clock.is_due(resumed_at));
        for sequence in 4..8 {
            ring.insert(parse_packet(&packet(sequence, u64::from(sequence) * 128, 0.5)).unwrap());
        }
        clock.prime_if_ready(&mut ring, resumed_at, startup_delay);
        assert!(!clock.is_due(resumed_at));
        assert!(clock.is_due(resumed_at + startup_delay));
    }

    #[test]
    fn target_delay_absorbs_burst_arrival_jitter_without_silence() {
        let period = Duration::from_micros(2_667);
        let startup_delay = period * 4;
        let epoch = Instant::now();
        let mut clock = PlayoutClock::default();
        let mut ring = PacketRing::new(512, 4096);

        for sequence in 0..4 {
            ring.insert(parse_packet(&packet(sequence, u64::from(sequence) * 128, 0.25)).unwrap());
        }
        clock.prime_if_ready(&mut ring, epoch, startup_delay);

        // The next four-packet callback is delayed by one packet period. A
        // target-sized startup hold leaves enough real audio to absorb it.
        for tick in 4..12u32 {
            let now = epoch + period * tick;
            assert!(clock.is_due(now));
            let (_, silence) = ring.pop_or_silence().unwrap();
            assert!(!silence, "unexpected silence at playout tick {tick}");
            clock.advance(period);
            if tick == 5 {
                for sequence in 4..8 {
                    ring.insert(
                        parse_packet(&packet(sequence, u64::from(sequence) * 128, 0.5)).unwrap(),
                    );
                }
            } else if tick == 9 {
                for sequence in 8..12 {
                    ring.insert(
                        parse_packet(&packet(sequence, u64::from(sequence) * 128, 0.75)).unwrap(),
                    );
                }
            }
        }
    }

    #[test]
    fn sequence_tracker_distinguishes_gap_from_reorder_and_wraps() {
        let mut tracker = SequenceTracker::default();
        assert_eq!(tracker.observe(100), SequenceResult::InOrder);
        assert_eq!(tracker.observe(101), SequenceResult::InOrder);
        assert_eq!(tracker.observe(103), SequenceResult::Gap);
        assert_eq!(tracker.observe(102), SequenceResult::OutOfOrder);
        tracker.reset();
        assert_eq!(tracker.observe(u32::MAX), SequenceResult::InOrder);
        assert_eq!(tracker.observe(0), SequenceResult::InOrder);
    }

    #[test]
    fn audio_health_uses_healthy_degraded_and_lost_thresholds() {
        let now = Instant::now();
        let loss = Duration::from_millis(250);
        assert_eq!(classify_audio_health(None, now, loss), AudioHealth::Waiting);
        assert_eq!(
            classify_audio_health(Some(now - Duration::from_millis(49)), now, loss),
            AudioHealth::Healthy
        );
        assert_eq!(
            classify_audio_health(Some(now - Duration::from_millis(50)), now, loss),
            AudioHealth::Degraded
        );
        assert_eq!(
            classify_audio_health(Some(now - Duration::from_millis(251)), now, loss),
            AudioHealth::Lost
        );
    }

    #[test]
    fn status_is_valid_json() {
        let status = SatpStatus::new(&BridgeConfig::default());
        let json = status_json(&status);
        assert!(json.starts_with("{\n"));
        assert!(json.ends_with("}\n"));
        assert!(json.contains("\"sink\": \"null\""));
        assert!(json.contains("\"buffer_target\": 512"));
    }
}
