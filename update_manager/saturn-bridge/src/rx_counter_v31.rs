//! RXC1 snapshots are acquired only by the active direct-XDMA receive owner.
//! The installed 1.31.001 image has no RXC1 bank; that is unavailable, not zero.

use crate::xdma::XdmaRegisterDevice;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BASE: u64 = 0x8000;
const MAGIC: u32 = 0x5258_4331;
const CANDIDATE_BUILD_ID: u32 = 0x5346_0004;
const REQUEST: u32 = 1 << 31;
const ACK: u32 = 1 << 30;
const VALID: u32 = 1;
const OVERFLOW: u32 = 1 << 2;
const RESET: u32 = 1 << 3;
const EXHAUSTED: u32 = (1 << 4) | (1 << 5);
const TOKEN_BOUND: u32 = 1 << 6;
const DDC_COUNT: usize = 10;
const PERIOD: Duration = Duration::from_secs(5);
static TOKEN_SEQUENCE: AtomicU32 = AtomicU32::new(1);

pub(crate) trait Registers {
    fn read(&self, address: u64) -> Result<u32, String>;
    fn write(&self, address: u64, value: u32) -> Result<(), String>;
}

impl Registers for XdmaRegisterDevice {
    fn read(&self, address: u64) -> Result<u32, String> {
        self.read_register(address)
            .map_err(|error| error.to_string())
    }

    fn write(&self, address: u64, value: u32) -> Result<(), String> {
        self.write_register(address, value)
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reading {
    pub(crate) receiver: usize,
    pub(crate) status: &'static str,
    pub(crate) snapshot_serial: u32,
    pub(crate) host_token: u32,
    pub(crate) session_generation: u32,
    pub(crate) configuration_generation: u32,
    pub(crate) observed_configuration_word: u64,
    pub(crate) rate_code: u32,
    pub(crate) counters: [u64; 5],
    pub(crate) overflow: bool,
}

impl Reading {
    fn invalid(receiver: usize, status: &'static str) -> Self {
        Self {
            receiver,
            status,
            snapshot_serial: 0,
            host_token: 0,
            session_generation: 0,
            configuration_generation: 0,
            observed_configuration_word: 0,
            rate_code: 0,
            counters: [0; 5],
            overflow: false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Owner {
    poll_enabled: bool,
    token: u32,
    session: Option<u32>,
    readings: Vec<Reading>,
    failures: u64,
    sampled_at_ms: u64,
    last_poll: Option<Instant>,
}

impl Owner {
    pub(crate) fn for_build_id(build_id: u32) -> Self {
        let setting = std::env::var("SATURN_RXC1_POLL_ENABLED").ok();
        Self::with_config(build_id, setting.as_deref())
    }

    fn with_config(build_id: u32, setting: Option<&str>) -> Self {
        // The 0x53460003 baseline has no safe RXC1 bank. Never probe a BAR
        // register merely to discover that it is absent.
        let requested = setting == Some("1");
        let enabled = requested && build_id == CANDIDATE_BUILD_ID;
        let status = if enabled {
            "unavailable"
        } else if requested {
            "unsupported"
        } else {
            "disabled"
        };
        Self::with_status(enabled, status)
    }

    #[cfg(test)]
    fn with_polling(poll_enabled: bool) -> Self {
        Self::with_status(
            poll_enabled,
            if poll_enabled {
                "unavailable"
            } else {
                "disabled"
            },
        )
    }

    fn with_status(poll_enabled: bool, status: &'static str) -> Self {
        Self {
            poll_enabled,
            token: 0,
            session: None,
            readings: (0..DDC_COUNT)
                .map(|receiver| Reading::invalid(receiver, status))
                .collect(),
            failures: 0,
            sampled_at_ms: 0,
            last_poll: None,
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.poll_enabled
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn new_token() -> u32 {
    let sequence = TOKEN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mixed = (nanos as u32)
        ^ ((nanos >> 32) as u32).rotate_left(11)
        ^ std::process::id().rotate_left(19)
        ^ sequence.wrapping_mul(0x9e37_79b9);
    mixed.max(1)
}

fn read(io: &impl Registers, offset: u64) -> Result<u32, &'static str> {
    io.read(BASE + offset).map_err(|_| "read_error")
}

fn write(io: &impl Registers, offset: u64, value: u32) -> Result<(), &'static str> {
    io.write(BASE + offset, value).map_err(|_| "write_error")
}

fn read64(io: &impl Registers, offset: u64) -> Result<u64, &'static str> {
    let low = u64::from(read(io, offset)?);
    let high = u64::from(read(io, offset + 4)?);
    Ok(low | (high << 32))
}

impl Owner {
    fn invalidate_all(&mut self, reason: &'static str) {
        self.readings = (0..DDC_COUNT)
            .map(|receiver| Reading::invalid(receiver, reason))
            .collect();
        self.sampled_at_ms = 0;
        if reason != "unsupported" {
            self.failures = self.failures.saturating_add(1);
        }
    }

    fn bind(&mut self, io: &impl Registers) -> Result<(), &'static str> {
        if read(io, 0x00)? != MAGIC {
            return Err("unsupported");
        }
        let status = read(io, 0x08)?;
        if status & RESET != 0 {
            return Err("reset_active");
        }
        if status & EXHAUSTED != 0 {
            return Err("generation_exhausted");
        }
        let session = read(io, 0x0c)?;
        let _serial = read(io, 0x10)?;
        let old_token = read(io, 0x4c)?;
        if status & VALID != 0 {
            write(io, 0x04, ACK).map_err(|_| "ack_failed")?;
            if read(io, 0x08)? & VALID != 0 {
                return Err("ack_failed");
            }
        }
        let mut token = new_token();
        if token == old_token {
            token = old_token.wrapping_add(1).max(1);
        }
        write(io, 0x4c, token)?;
        if read(io, 0x4c)? != token {
            return Err("token_mismatch");
        }
        if read(io, 0x0c)? != session || read(io, 0x08)? & RESET != 0 {
            return Err("reset_changed");
        }
        self.token = token;
        self.session = Some(session);
        Ok(())
    }

    fn acquire(&self, io: &impl Registers, receiver: usize) -> Result<Reading, &'static str> {
        let before_status = read(io, 0x08)?;
        if before_status & RESET != 0 {
            return Err("reset_active");
        }
        if before_status & EXHAUSTED != 0 {
            return Err("generation_exhausted");
        }
        if before_status & VALID != 0 {
            return Err("stale_snapshot");
        }
        if before_status & TOKEN_BOUND == 0 || read(io, 0x4c)? != self.token {
            return Err("token_mismatch");
        }
        let session = read(io, 0x0c)?;
        if Some(session) != self.session {
            return Err("reset_changed");
        }
        let previous_serial = read(io, 0x10)?;
        if previous_serial == u32::MAX {
            return Err("generation_exhausted");
        }
        write(io, 0x04, REQUEST | receiver as u32)?;
        let result = (|| {
            let status = read(io, 0x08)?;
            if status & RESET != 0 {
                return Err("reset_changed");
            }
            if status & EXHAUSTED != 0 {
                return Err("generation_exhausted");
            }
            if status & VALID == 0 {
                return Err("snapshot_unavailable");
            }
            let serial = read(io, 0x10)?;
            if serial != previous_serial + 1 {
                return Err("stale_serial");
            }
            let frozen_session = read(io, 0x14)?;
            if frozen_session != session {
                return Err("reset_changed");
            }
            let configuration_generation = read(io, 0x18)?;
            if configuration_generation == 0 {
                return Err("configuration_mismatch");
            }
            let metadata = read(io, 0x1c)?;
            if metadata & 0xFFFF_FF80 != 0 || metadata & 0x0f != receiver as u32 {
                return Err("receiver_mismatch");
            }
            let frozen_token = read(io, 0x50)?;
            if frozen_token != self.token {
                return Err("token_mismatch");
            }
            let configuration = read64(io, 0x54)?;
            if configuration >> 39 != 0 {
                return Err("configuration_mismatch");
            }
            let mut counters = [0; 5];
            for (index, counter) in counters.iter_mut().enumerate() {
                *counter = read64(io, 0x20 + index as u64 * 8)?;
            }
            let frozen_overflow = read(io, 0x48)?;
            if frozen_overflow > 1 || (status & OVERFLOW != 0) != (frozen_overflow != 0) {
                return Err("overflow_mismatch");
            }
            let end_status = read(io, 0x08)?;
            if end_status & RESET != 0 || end_status & VALID == 0 {
                return Err("reset_changed");
            }
            if end_status != status
                || read(io, 0x10)? != serial
                || read(io, 0x0c)? != session
                || read(io, 0x14)? != frozen_session
                || read(io, 0x18)? != configuration_generation
                || read(io, 0x1c)? != metadata
                || read(io, 0x50)? != frozen_token
                || read(io, 0x4c)? != self.token
                || read64(io, 0x54)? != configuration
                || read(io, 0x48)? != frozen_overflow
            {
                return Err("snapshot_changed");
            }
            Ok(Reading {
                receiver,
                status: "valid",
                snapshot_serial: serial,
                host_token: self.token,
                session_generation: session,
                configuration_generation,
                observed_configuration_word: configuration,
                rate_code: (metadata >> 4) & 7,
                counters,
                overflow: frozen_overflow != 0,
            })
        })();
        // A successful ACK alone is not evidence of a post-reset reading.
        if write(io, 0x04, ACK).is_err() {
            return Err("ack_failed");
        }
        let after_ack = read(io, 0x08)?;
        if after_ack & RESET != 0 {
            return Err("reset_changed");
        }
        if after_ack & VALID != 0 {
            return Err("ack_failed");
        }
        if after_ack & TOKEN_BOUND == 0 {
            return Err("token_mismatch");
        }
        if let Ok(ref reading) = result {
            if read(io, 0x0c)? != reading.session_generation
                || read(io, 0x10)? != reading.snapshot_serial
                || read(io, 0x4c)? != reading.host_token
            {
                return Err("reset_changed");
            }
        }
        result
    }

    pub(crate) fn maybe_sample(&mut self, io: &impl Registers) {
        if !self.poll_enabled {
            return;
        }
        if self.last_poll.is_some_and(|last| last.elapsed() < PERIOD) {
            return;
        }
        self.last_poll = Some(Instant::now());
        if self.session.is_none() {
            if let Err(reason) = self.bind(io) {
                self.invalidate_all(reason);
                return;
            }
        }
        let mut readings = Vec::with_capacity(DDC_COUNT);
        for receiver in 0..DDC_COUNT {
            match self.acquire(io, receiver) {
                Ok(reading) => readings.push(reading),
                Err(reason) => {
                    self.failures = self.failures.saturating_add(1);
                    readings.push(Reading::invalid(receiver, reason));
                    if matches!(
                        reason,
                        "reset_changed"
                            | "reset_active"
                            | "token_mismatch"
                            | "generation_exhausted"
                            | "ack_failed"
                            | "write_error"
                            | "stale_snapshot"
                            | "snapshot_unavailable"
                    ) {
                        self.session = None;
                        readings = (0..DDC_COUNT)
                            .map(|ddc| Reading::invalid(ddc, reason))
                            .collect();
                        // A request write can reach the FPGA even when its host
                        // completion reports an error. Discard this entire poll,
                        // then make one checked, bounded recovery attempt. bind()
                        // ACKs any pending snapshot, verifies it cleared, and
                        // installs a fresh token. Never restart the radio here.
                        if matches!(
                            reason,
                            "write_error" | "stale_snapshot" | "snapshot_unavailable"
                        ) {
                            let _ = self.bind(io);
                        }
                        break;
                    }
                }
            }
        }
        self.sampled_at_ms = if readings.iter().any(|reading| reading.status == "valid") {
            now_ms()
        } else {
            0
        };
        self.readings = readings;
    }

    pub(crate) fn json(&self) -> String {
        let valid = self
            .readings
            .iter()
            .filter(|reading| reading.status == "valid")
            .count();
        let status = if valid == DDC_COUNT {
            "valid"
        } else if valid != 0 {
            "partial"
        } else {
            self.readings
                .first()
                .map_or("unavailable", |reading| reading.status)
        };
        let mut output = format!(
            "{{\"schema\":\"rxc1-v1\",\"source_backend\":\"xdma\",\"status\":\"{status}\",\"sampled_at_ms\":{},\"host_acquisition_failures\":{},\"ddc\":[",
            self.sampled_at_ms, self.failures
        );
        for (index, reading) in self.readings.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            let _ = write!(
                output,
                "{{\"receiver\":{},\"status\":\"{}\"",
                reading.receiver, reading.status
            );
            if reading.status == "valid" {
                let _ = write!(output,
                    ",\"snapshot_serial\":{},\"host_token\":{},\"session_generation\":{},\"configuration_generation\":{},\"observed_configuration_word\":\"0x{:010x}\",\"rate_code\":{},\"overflow\":{},\"exact\":{}",
                    reading.snapshot_serial, reading.host_token, reading.session_generation,
                    reading.configuration_generation, reading.observed_configuration_word,
                    reading.rate_code, reading.overflow, !reading.overflow);
            } else {
                output.push_str(",\"exact\":false");
            }
            let fields = [
                "accepted_pre_fir_pairs",
                "refused_pre_fir_pair_candidates",
                "accepted_clamped_iq_components",
                "partial_ready_anomalies",
                "one_sided_valid_anomalies",
            ];
            for (field, value) in fields.iter().zip(reading.counters) {
                if reading.status == "valid" {
                    let _ = write!(output, ",\"{field}\":\"{value}\"");
                } else {
                    let _ = write!(output, ",\"{field}\":null");
                }
            }
            output.push('}');
        }
        output.push_str("]}");
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn disabled_polling_never_touches_registers_and_never_reports_zero() {
        struct NoAccess;
        impl Registers for NoAccess {
            fn read(&self, _: u64) -> Result<u32, String> {
                panic!("disabled RXC1 reader accessed registers")
            }
            fn write(&self, _: u64, _: u32) -> Result<(), String> {
                panic!("disabled RXC1 reader accessed registers")
            }
        }
        let mut owner = Owner::with_polling(false);
        owner.maybe_sample(&NoAccess);
        assert!(!owner.is_enabled());
        assert!(owner
            .readings
            .iter()
            .all(|reading| reading.status == "disabled"));
        let json = owner.json();
        assert!(json.contains("\"status\":\"disabled\""));
        assert!(json.contains("\"refused_pre_fir_pair_candidates\":null"));
    }

    #[test]
    fn baseline_and_default_never_probe_rx_counter_bank() {
        struct NoAccess;
        impl Registers for NoAccess {
            fn read(&self, _: u64) -> Result<u32, String> {
                panic!("unsafe RXC1 read on baseline or default configuration")
            }
            fn write(&self, _: u64, _: u32) -> Result<(), String> {
                panic!("unsafe RXC1 write on baseline or default configuration")
            }
        }
        for (build_id, setting, expected) in [
            (CANDIDATE_BUILD_ID, None, "disabled"),
            (CANDIDATE_BUILD_ID, Some("0"), "disabled"),
            (CANDIDATE_BUILD_ID, Some("true"), "disabled"),
            (0x5346_0003, Some("1"), "unsupported"),
            (0, Some("1"), "unsupported"),
        ] {
            let mut owner = Owner::with_config(build_id, setting);
            owner.maybe_sample(&NoAccess);
            assert!(!owner.is_enabled());
            assert!(owner
                .readings
                .iter()
                .all(|reading| reading.status == expected));
            assert_eq!(owner.failures, 0);
        }
        assert!(Owner::with_config(CANDIDATE_BUILD_ID, Some("1")).is_enabled());
    }

    // The expected values are the independently scripted snapshot-boundary
    // oracle. The fake bank may corrupt its readback without changing these.
    const EXPECTED: [[u64; 5]; DDC_COUNT] = [
        [100, 3, 7, 1, 2],
        [200, 9, 11, 2, 3],
        [300, 15, 17, 3, 4],
        [400, 21, 23, 4, 5],
        [500, 27, 29, 5, 6],
        [600, 33, 35, 6, 7],
        [700, 39, 41, 7, 8],
        [800, 45, 47, 8, 9],
        [900, 51, 53, 9, 10],
        [1000, 57, 59, 10, 11],
    ];

    #[derive(Clone, Copy, Default)]
    enum Corruption {
        #[default]
        None,
        Counter,
        Receiver,
        Configuration,
        Token,
        Reset,
        ResetOnAck,
        ResetCompletedOnAck,
        SessionChangedOnAck,
        SerialChangedOnAck,
        TokenChangedOnAck,
        RequestAcceptedWriteError,
        ReadError,
        StaleSerial,
    }

    #[derive(Default)]
    struct State {
        token: u32,
        serial: u32,
        session: u32,
        selected: usize,
        valid: bool,
        reset: bool,
        saturated: bool,
        corruption: Corruption,
        corrupt_target: Option<(usize, usize)>,
        frozen_config_reads: u32,
        fault_fired: bool,
        failed_request_token: u32,
        ack_failures_remaining: u32,
        ack_attempts: u32,
    }

    #[derive(Default)]
    struct Fake(RefCell<State>);

    impl Registers for Fake {
        fn read(&self, address: u64) -> Result<u32, String> {
            let mut state = self.0.borrow_mut();
            let offset = address - BASE;
            if state.valid
                && state.selected == 6
                && matches!(state.corruption, Corruption::Reset)
                && offset == 0x30
            {
                state.valid = false;
                state.reset = true;
                state.session += 1;
                state.serial = 0;
            }
            if state.valid && matches!(state.corruption, Corruption::ReadError) && offset == 0x30 {
                return Err("injected short read".to_string());
            }
            if (0x14..=0x48).contains(&offset) || matches!(offset, 0x50 | 0x54 | 0x58) {
                if !state.valid || state.reset {
                    return Err("invalid frozen read".to_string());
                }
            }
            let receiver = state.selected;
            let value = match offset {
                0x00 => MAGIC,
                0x08 => {
                    u32::from(state.valid)
                        | (u32::from(state.reset) << 3)
                        | (u32::from(state.valid && state.saturated) << 2)
                        | (u32::from(state.token != 0) << 6)
                }
                0x0c | 0x14 => state.session,
                0x10 => state.serial,
                0x18 => {
                    state.frozen_config_reads += 1;
                    if matches!(state.corruption, Corruption::Configuration)
                        && state.frozen_config_reads >= 2
                    {
                        2
                    } else {
                        1
                    }
                }
                0x1c => {
                    0x40 | if matches!(state.corruption, Corruption::Receiver) {
                        ((receiver + 1) % DDC_COUNT) as u32
                    } else {
                        receiver as u32
                    }
                }
                0x20..=0x44 if offset % 4 == 0 => {
                    let index = ((offset - 0x20) / 8) as usize;
                    let mut counter = EXPECTED[receiver][index];
                    if state.saturated && receiver == 0 && index == 1 {
                        counter = u64::MAX;
                    }
                    if matches!(state.corruption, Corruption::Counter)
                        && state.corrupt_target == Some((receiver, index))
                    {
                        counter += 1;
                    }
                    if offset % 8 == 0 {
                        counter as u32
                    } else {
                        (counter >> 32) as u32
                    }
                }
                0x48 => u32::from(state.saturated),
                0x4c => state.token,
                0x50 => {
                    if matches!(state.corruption, Corruption::Token) {
                        state.token ^ 1
                    } else {
                        state.token
                    }
                }
                0x54 => receiver as u32 + 1,
                0x58 => 0,
                _ => return Err(format!("unknown offset 0x{offset:x}")),
            };
            Ok(value)
        }

        fn write(&self, address: u64, value: u32) -> Result<(), String> {
            let mut state = self.0.borrow_mut();
            if state.reset {
                return Err("reset active".to_string());
            }
            match address - BASE {
                0x4c if !state.valid && value != 0 => state.token = value,
                0x04 if value == ACK && state.valid => {
                    state.ack_attempts += 1;
                    if state.ack_failures_remaining != 0 {
                        state.ack_failures_remaining -= 1;
                        return Err("injected ACK completion failure".to_string());
                    }
                    state.valid = false;
                    if state.selected == 6 && matches!(state.corruption, Corruption::ResetOnAck) {
                        state.reset = true;
                        state.session += 1;
                        state.serial = 0;
                    }
                    if state.selected == DDC_COUNT - 1 {
                        match state.corruption {
                            Corruption::ResetCompletedOnAck => {
                                state.session += 1;
                                state.serial = 0;
                                state.reset = false;
                            }
                            Corruption::SessionChangedOnAck => state.session += 1,
                            Corruption::SerialChangedOnAck => state.serial += 1,
                            Corruption::TokenChangedOnAck => state.token ^= 1,
                            _ => {}
                        }
                    }
                }
                0x04 if value & REQUEST != 0 && !state.valid && state.token != 0 => {
                    state.selected = (value & 0x0f) as usize;
                    state.valid = true;
                    state.frozen_config_reads = 0;
                    if !matches!(state.corruption, Corruption::StaleSerial) {
                        state.serial += 1;
                    }
                    if matches!(state.corruption, Corruption::RequestAcceptedWriteError)
                        && !state.fault_fired
                    {
                        state.fault_fired = true;
                        state.failed_request_token = state.token;
                        return Err("request accepted but completion failed".to_string());
                    }
                }
                _ => return Err("invalid write".to_string()),
            }
            Ok(())
        }
    }

    fn sample(fake: &impl Registers) -> Owner {
        let mut owner = Owner::with_polling(true);
        owner.maybe_sample(fake);
        owner
    }

    #[test]
    fn checks_every_counter_against_independent_nonzero_boundary_oracle() {
        let fake = Fake::default();
        let owner = sample(&fake);
        assert_eq!(owner.readings.len(), DDC_COUNT);
        for (receiver, reading) in owner.readings.iter().enumerate() {
            assert_eq!(reading.status, "valid");
            assert_eq!(reading.receiver, receiver);
            assert_eq!(reading.counters, EXPECTED[receiver]);
            assert_eq!(reading.snapshot_serial, receiver as u32 + 1);
            assert_eq!(reading.session_generation, 0);
            assert_eq!(reading.configuration_generation, 1);
            assert_eq!(reading.observed_configuration_word, receiver as u64 + 1);
            assert_eq!(reading.rate_code, 4);
            assert_ne!(reading.host_token, 0);
        }
        assert!(owner
            .json()
            .contains("\"refused_pre_fir_pair_candidates\":\"57\""));
    }

    #[test]
    fn each_adversarial_corruption_is_detected_by_transaction_or_boundary_oracle() {
        for corruption in [
            Corruption::Counter,
            Corruption::Receiver,
            Corruption::Configuration,
            Corruption::Token,
            Corruption::Reset,
            Corruption::ResetOnAck,
            Corruption::ResetCompletedOnAck,
            Corruption::SessionChangedOnAck,
            Corruption::SerialChangedOnAck,
            Corruption::TokenChangedOnAck,
            Corruption::ReadError,
            Corruption::StaleSerial,
        ] {
            let fake = Fake::default();
            {
                let mut state = fake.0.borrow_mut();
                state.corruption = corruption;
                if matches!(corruption, Corruption::Counter) {
                    state.corrupt_target = Some((3, 1));
                }
            }
            let owner = sample(&fake);
            let software_rejected = owner
                .readings
                .iter()
                .any(|reading| reading.status != "valid");
            let oracle_rejected = owner
                .readings
                .iter()
                .enumerate()
                .any(|(receiver, reading)| {
                    reading.status == "valid" && reading.counters != EXPECTED[receiver]
                });
            if matches!(corruption, Corruption::Counter) {
                assert!(
                    !software_rejected,
                    "plausible counter corruption must reach the oracle"
                );
                assert!(
                    oracle_rejected,
                    "independent boundary oracle must reject payload"
                );
            } else {
                assert!(
                    software_rejected,
                    "metadata/read/reset corruption must fail in owner"
                );
            }
            if software_rejected {
                assert!(owner.json().contains("null"));
            }
            if matches!(
                corruption,
                Corruption::Reset
                    | Corruption::ResetOnAck
                    | Corruption::ResetCompletedOnAck
                    | Corruption::SessionChangedOnAck
                    | Corruption::SerialChangedOnAck
                    | Corruption::TokenChangedOnAck
            ) {
                assert!(owner
                    .readings
                    .iter()
                    .all(|reading| reading.status != "valid"));
            }
        }
    }

    #[test]
    fn owner_restart_acknowledges_old_snapshot_and_binds_a_new_token() {
        let fake = Fake::default();
        let first = sample(&fake);
        let first_token = first.token;
        fake.write(BASE + 0x04, REQUEST).unwrap();
        assert!(fake.0.borrow().valid);
        let second = sample(&fake);
        assert!(second
            .readings
            .iter()
            .all(|reading| reading.status == "valid"));
        assert_ne!(second.token, first_token);
        assert!(!fake.0.borrow().valid);
    }

    #[test]
    fn accepted_request_with_failed_completion_recovers_without_publishing() {
        let fake = Fake::default();
        fake.0.borrow_mut().corruption = Corruption::RequestAcceptedWriteError;
        let mut owner = sample(&fake);
        assert!(fake.0.borrow().fault_fired);
        assert_ne!(owner.token, fake.0.borrow().failed_request_token);
        assert!(
            !fake.0.borrow().valid,
            "checked recovery must ACK pending data"
        );
        assert!(owner
            .readings
            .iter()
            .all(|reading| reading.status == "write_error"));
        assert_eq!(owner.sampled_at_ms, 0);
        let rebound_token = owner.token;
        owner.last_poll = None;
        owner.maybe_sample(&fake);
        assert!(owner
            .readings
            .iter()
            .all(|reading| reading.status == "valid"));
        assert_eq!(owner.readings[0].host_token, rebound_token);
        assert!(!fake.0.borrow().valid);
    }

    #[test]
    fn unexpected_pending_snapshot_is_acked_and_rebound() {
        let fake = Fake::default();
        let mut owner = sample(&fake);
        let old_token = owner.token;
        fake.write(BASE + 0x04, REQUEST).unwrap();
        assert!(fake.0.borrow().valid);
        owner.last_poll = None;
        owner.maybe_sample(&fake);
        assert!(owner
            .readings
            .iter()
            .all(|reading| reading.status == "stale_snapshot"));
        assert_eq!(owner.sampled_at_ms, 0);
        assert!(!fake.0.borrow().valid);
        assert_ne!(owner.token, old_token);
        owner.last_poll = None;
        owner.maybe_sample(&fake);
        assert!(owner
            .readings
            .iter()
            .all(|reading| reading.status == "valid"));
    }

    #[test]
    fn failed_recovery_is_bounded_and_retried_by_the_owner() {
        let fake = Fake::default();
        {
            let mut state = fake.0.borrow_mut();
            state.corruption = Corruption::RequestAcceptedWriteError;
            state.ack_failures_remaining = 1;
        }
        let mut owner = sample(&fake);
        assert!(owner
            .readings
            .iter()
            .all(|reading| reading.status == "write_error"));
        assert_eq!(owner.sampled_at_ms, 0);
        assert!(fake.0.borrow().valid);
        assert_eq!(fake.0.borrow().ack_attempts, 1);
        assert!(owner.session.is_none());
        owner.last_poll = None;
        owner.maybe_sample(&fake);
        assert!(owner
            .readings
            .iter()
            .all(|reading| reading.status == "valid"));
        assert!(!fake.0.borrow().valid);
        assert_ne!(owner.token, fake.0.borrow().failed_request_token);
    }

    #[test]
    fn independent_oracle_rejects_each_counter_in_each_receiver() {
        for receiver in 0..DDC_COUNT {
            for counter in 0..5 {
                let fake = Fake::default();
                {
                    let mut state = fake.0.borrow_mut();
                    state.corruption = Corruption::Counter;
                    state.corrupt_target = Some((receiver, counter));
                }
                let owner = sample(&fake);
                assert!(owner
                    .readings
                    .iter()
                    .all(|reading| reading.status == "valid"));
                let discrepancies = owner
                    .readings
                    .iter()
                    .enumerate()
                    .flat_map(|(ddc, reading)| {
                        reading
                            .counters
                            .iter()
                            .enumerate()
                            .filter_map(move |(field, value)| {
                                (value != &EXPECTED[ddc][field]).then_some((ddc, field))
                            })
                    })
                    .collect::<Vec<_>>();
                assert_eq!(discrepancies, [(receiver, counter)]);
            }
        }
    }

    #[test]
    fn unsupported_image_and_reset_never_publish_zero_counters() {
        struct Missing;
        impl Registers for Missing {
            fn read(&self, _: u64) -> Result<u32, String> {
                Ok(0)
            }
            fn write(&self, _: u64, _: u32) -> Result<(), String> {
                Ok(())
            }
        }
        let missing = sample(&Missing);
        assert_eq!(missing.readings[0].status, "unsupported");
        assert!(missing.json().contains("\"accepted_pre_fir_pairs\":null"));
        let fake = Fake::default();
        fake.0.borrow_mut().reset = true;
        let reset = sample(&fake);
        assert_eq!(reset.readings[0].status, "reset_active");
        assert!(reset
            .json()
            .contains("\"refused_pre_fir_pair_candidates\":null"));
    }

    #[test]
    fn saturation_is_published_as_inexact_without_truncating_u64() {
        let fake = Fake::default();
        fake.0.borrow_mut().saturated = true;
        let owner = sample(&fake);
        assert_eq!(owner.readings[0].counters[1], u64::MAX);
        assert!(owner.readings[0].overflow);
        assert!(owner
            .json()
            .contains("\"refused_pre_fir_pair_candidates\":\"18446744073709551615\""));
        assert!(owner.json().contains("\"exact\":false"));
    }
}
