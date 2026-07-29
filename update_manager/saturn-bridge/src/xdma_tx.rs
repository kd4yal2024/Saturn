//! Phase 5 guarded direct-XDMA transmit validation.
//!
//! The preflight remains completely RF-inhibited. The separate RF probe is
//! deliberately locked to the first field-validation envelope: Saturn PCB2
//! firmware 1.27, ANT1, 7.200 MHz, and a 3 W absolute ceiling.

use crate::xdma::{ensure_p2app_inactive, XdmaError, XdmaRegisterDevice};
use crate::xdma_duc::{
    allowed_cpu_ids, current_scheduler, enable_realtime_fifo, pin_current_thread, DucDmaSession,
};
use crate::xdma_telemetry::{record_probe_outcome, TelemetryValue};
use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const FAILURE_INJECTION_ENV: &str = "SATURN_BRIDGE_XDMA_TX_PREFLIGHT_INJECT_FAILURE";
const TX_CONFIRM_ENV: &str = "SATURN_BRIDGE_XDMA_TX_CONFIRM";
const TX_CONFIRM_TOKEN: &str = "ANTENNA_CONNECTED_40M_7200000HZ_3W_ANT1";
const TX_FREQUENCY_ENV: &str = "SATURN_BRIDGE_XDMA_TX_FREQUENCY_HZ";
const TX_MAX_WATTS_ENV: &str = "SATURN_BRIDGE_XDMA_TX_MAX_WATTS";
const TX_ANTENNA_ENV: &str = "SATURN_BRIDGE_XDMA_TX_ANTENNA";
const TX_DURATION_ENV: &str = "SATURN_BRIDGE_XDMA_TX_DURATION_MS";
const TX_POWER_SCALE_ENV: &str = "SATURN_BRIDGE_XDMA_TX_POWER_METER_SCALE";

const VALIDATED_FREQUENCY_HZ: u32 = 7_200_000;
const VALIDATED_MAX_WATTS: f32 = 3.0;
const VALIDATED_ANTENNA: u8 = 1;
const DEFAULT_TX_DURATION_MS: u64 = 250;
const MIN_TX_DURATION_MS: u64 = 100;
const MAX_TX_DURATION_MS: u64 = 500;
const TX_TARGET_WATTS: f32 = 2.5;
const REVERSE_POWER_TRIP_WATTS: f32 = 0.75;
const SWR_TRIP: f32 = 3.0;
const SWR_MIN_FORWARD_WATTS: f32 = 0.25;
const MIN_MEASURABLE_FORWARD_WATTS: f32 = 0.05;
const DRIVE_STEP_INTERVAL: Duration = Duration::from_millis(12);
const TX_LOOP_SLEEP: Duration = Duration::from_micros(250);
const REQUIRED_COMPLETION_PRIORITY: i32 = 20;
const TX_RT_PRIORITY: i32 = 20;

const TX_CONFIG_REGISTER: u64 = 0x2008;
const TX_DUC_REGISTER: u64 = 0x200c;
const RF_GPIO_REGISTER: u64 = 0x2014;
const DAC_CONTROL_REGISTER: u64 = 0x201c;
const ALEX_FORWARD_POWER_REGISTER: u64 = 0xa000;
const ALEX_REVERSE_POWER_REGISTER: u64 = 0xa004;
const ALEX_EXCITER_POWER_REGISTER: u64 = 0xa010;
const ALEX_TX_FILTER_REGISTER: u64 = 0xb000;
const ALEX_TX_ANTENNA_REGISTER: u64 = 0xb008;

const MOX_BIT: u32 = 1 << 24;
const TX_ENABLE_BIT: u32 = 1 << 25;
const RF_DATA_NETWORK_ENDIAN_BIT: u32 = 1 << 26;
const TX_RELAY_DISABLE_BIT: u32 = 1 << 27;
const TX_MODULATION_SOURCE_MASK: u32 = 0b11;
const TX_OUTPUT_GATE_BIT: u32 = 1 << 2;
const TX_PROTOCOL_P2_BIT: u32 = 1 << 3;
const TX_AMPLITUDE_MASK: u32 = 0x3ffff << 4;
const TX_WATCHDOG_OVERRIDE_BIT: u32 = 1 << 28;
const DUC_MUX_RESET_BIT: u32 = 1 << 29;
const TX_IQ_DEINTERLEAVE_BIT: u32 = 1 << 30;
const DUC_STREAM_ENABLE_BIT: u32 = 1 << 31;
const PCB2_FW13_TX_AMPLITUDE: u32 = 0x2000;
const ALEX_60_40M_LPF_BIT: u16 = 0x0020;
const ALEX_ANT1_BIT: u16 = 0x0100;
const ALEX_TX_RELAY_BIT: u16 = 0x0800;

static TX_STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailureInjection {
    None,
    AfterOpen,
    AfterVerify,
}

#[derive(Clone, Copy, Debug)]
struct TxProbeConfig {
    duration: Duration,
    power_meter_scale: f32,
}

#[derive(Clone, Copy, Debug)]
struct PowerSample {
    exciter_raw: u16,
    forward_raw: u16,
    reverse_raw: u16,
    forward_watts: f32,
    reverse_watts: f32,
    swr: f32,
}

struct SignalGuard {
    previous_int: libc::sigaction,
    previous_term: libc::sigaction,
}

impl SignalGuard {
    fn install() -> Result<Self, XdmaError> {
        TX_STOP_REQUESTED.store(false, Ordering::SeqCst);
        // SAFETY: sigaction structures are initialized before use, the handler
        // only performs an atomic store, and the previous handlers are retained
        // for exact restoration when the bounded probe exits.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = tx_stop_signal as *const () as usize;
            libc::sigemptyset(&mut action.sa_mask);
            action.sa_flags = 0;
            let mut previous_int: libc::sigaction = std::mem::zeroed();
            let mut previous_term: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(libc::SIGINT, &action, &mut previous_int) != 0 {
                return Err(XdmaError::Io {
                    action: "could not install guarded TX SIGINT handler",
                    source: std::io::Error::last_os_error(),
                });
            }
            if libc::sigaction(libc::SIGTERM, &action, &mut previous_term) != 0 {
                libc::sigaction(libc::SIGINT, &previous_int, std::ptr::null_mut());
                return Err(XdmaError::Io {
                    action: "could not install guarded TX SIGTERM handler",
                    source: std::io::Error::last_os_error(),
                });
            }
            Ok(Self {
                previous_int,
                previous_term,
            })
        }
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        // SAFETY: these are the exact handler values returned by sigaction.
        unsafe {
            libc::sigaction(libc::SIGINT, &self.previous_int, std::ptr::null_mut());
            libc::sigaction(libc::SIGTERM, &self.previous_term, std::ptr::null_mut());
        }
    }
}

extern "C" fn tx_stop_signal(_signal: libc::c_int) {
    TX_STOP_REQUESTED.store(true, Ordering::SeqCst);
}

fn parse_failure_injection(value: Option<&str>) -> Result<FailureInjection, XdmaError> {
    match value {
        None => Ok(FailureInjection::None),
        Some("after-open") => Ok(FailureInjection::AfterOpen),
        Some("after-verify") => Ok(FailureInjection::AfterVerify),
        Some(value) => Err(XdmaError::Incompatible(format!(
            "{FAILURE_INJECTION_ENV} must be after-open or after-verify, not {value:?}"
        ))),
    }
}

fn injected_failure(checkpoint: &'static str) -> XdmaError {
    XdmaError::Incompatible(format!(
        "intentional Phase 5 TX preflight failure after {checkpoint}; emergency receive-safe cleanup was armed"
    ))
}

impl TxProbeConfig {
    fn from_env() -> Result<Self, XdmaError> {
        require_env(TX_CONFIRM_ENV, TX_CONFIRM_TOKEN)?;
        require_env(TX_FREQUENCY_ENV, &VALIDATED_FREQUENCY_HZ.to_string())?;
        require_env(TX_MAX_WATTS_ENV, "3")?;
        require_env(TX_ANTENNA_ENV, &VALIDATED_ANTENNA.to_string())?;
        let duration_ms = parse_env_u64(TX_DURATION_ENV, DEFAULT_TX_DURATION_MS)?;
        if !(MIN_TX_DURATION_MS..=MAX_TX_DURATION_MS).contains(&duration_ms) {
            return Err(XdmaError::Incompatible(format!(
                "{TX_DURATION_ENV} must be within {MIN_TX_DURATION_MS}..={MAX_TX_DURATION_MS} ms"
            )));
        }
        let power_meter_scale = parse_env_f32(TX_POWER_SCALE_ENV, 1.0)?;
        if !(0.5..=1.5).contains(&power_meter_scale) {
            return Err(XdmaError::Incompatible(format!(
                "{TX_POWER_SCALE_ENV} must be within 0.5..=1.5"
            )));
        }
        Ok(Self {
            duration: Duration::from_millis(duration_ms),
            power_meter_scale,
        })
    }
}

fn require_env(name: &'static str, expected: &str) -> Result<(), XdmaError> {
    match env::var(name) {
        Ok(value) if value == expected => Ok(()),
        Ok(value) => Err(XdmaError::Incompatible(format!(
            "{name} must be exactly {expected:?}, not {value:?}"
        ))),
        Err(_) => Err(XdmaError::Incompatible(format!(
            "{name}={expected:?} is required before the RF-generating probe can run"
        ))),
    }
}

fn parse_env_u64(name: &'static str, default: u64) -> Result<u64, XdmaError> {
    match env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|_| XdmaError::Incompatible(format!("{name} must be an integer"))),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(XdmaError::Incompatible(format!(
            "could not read {name}: {error}"
        ))),
    }
}

fn parse_env_f32(name: &'static str, default: f32) -> Result<f32, XdmaError> {
    match env::var(name) {
        Ok(value) => value
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| XdmaError::Incompatible(format!("{name} must be a finite number"))),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(XdmaError::Incompatible(format!(
            "could not read {name}: {error}"
        ))),
    }
}

pub fn run_phase5_tx_preflight() -> Result<(), XdmaError> {
    ensure_p2app_inactive()?;
    let injection = parse_failure_injection(env::var(FAILURE_INJECTION_ENV).ok().as_deref())?;
    let register_path = register_path();

    let registers = XdmaRegisterDevice::open(&register_path)?;
    if injection == FailureInjection::AfterOpen {
        return Err(injected_failure("register open"));
    }
    registers.verify_safe_receive_state()?;
    if injection == FailureInjection::AfterVerify {
        return Err(injected_failure("safe-state verification"));
    }

    let identity = registers.identity().clone();
    let device = registers.path().display().to_string();
    registers.close_safely()?;
    record_probe_outcome(
        5,
        "tx-preflight",
        "passed",
        "receive-safe-verified",
        None,
        &[
            ("device", TelemetryValue::text(device.clone())),
            ("product", TelemetryValue::number(identity.product_id)),
            ("pcb", TelemetryValue::number(identity.pcb_version)),
            (
                "firmware",
                TelemetryValue::text(format!(
                    "{}.{}",
                    identity.firmware_major, identity.firmware_minor
                )),
            ),
            ("rf_keyed", TelemetryValue::boolean(false)),
            ("amplitude_zero", TelemetryValue::boolean(true)),
            ("mox", TelemetryValue::boolean(false)),
            ("tx_enable", TelemetryValue::boolean(false)),
            ("pa_relay", TelemetryValue::boolean(false)),
            ("cw", TelemetryValue::boolean(false)),
        ],
    );
    println!(
        "saturn-bridge: XDMA Phase 5 TX preflight passed product={} pcb={} firmware={}.{} device={} rf_keyed=0 amplitude_zero=1 mox=0 tx_enable=0 pa_relay=0 cw=0",
        identity.product_id,
        identity.pcb_version,
        identity.firmware_major,
        identity.firmware_minor,
        device
    );
    println!(
        "saturn-bridge: XDMA Phase 5 preflight cleanup verified; no RF transmit operation was attempted"
    );
    Ok(())
}

pub fn run_phase5_tx_probe() -> Result<(), XdmaError> {
    ensure_p2app_inactive()?;
    let config = TxProbeConfig::from_env()?;
    verify_completion_policy()?;
    let cpu = allowed_cpu_ids()?.last().copied().ok_or_else(|| {
        XdmaError::Incompatible("no CPU is available for the guarded TX probe".into())
    })?;
    pin_current_thread(cpu)?;
    enable_realtime_fifo(TX_RT_PRIORITY)?;
    let (scheduler, scheduler_priority) = current_scheduler()?;
    let _signals = SignalGuard::install()?;
    let register_path = register_path();
    let duc_path = env::var_os("SATURN_BRIDGE_XDMA_DUC_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/xdma0_h2c_0"));

    let mut registers = XdmaRegisterDevice::open(&register_path)?;
    let identity = registers.identity().clone();
    if identity.is_fallback()
        || identity.pcb_version != 2
        || identity.firmware_major != 1
        || identity.firmware_minor != 27
    {
        return Err(XdmaError::Incompatible(format!(
            "guarded TX is validated only on the primary Saturn PCB2 firmware 1.27 image; found pcb={} firmware={}.{} image={}",
            identity.pcb_version,
            identity.firmware_major,
            identity.firmware_minor,
            if identity.is_fallback() { "fallback" } else { "primary" }
        )));
    }
    configure_safe_tx_baseline(&registers)?;
    let baseline = read_power(&registers, config.power_meter_scale)?;
    if baseline.forward_watts >= SWR_MIN_FORWARD_WATTS
        || baseline.reverse_watts >= SWR_MIN_FORWARD_WATTS
    {
        return Err(XdmaError::Incompatible(format!(
            "guarded TX baseline power is not near zero: forward={:.3}W reverse={:.3}W",
            baseline.forward_watts, baseline.reverse_watts
        )));
    }

    let mut session = DucDmaSession::start(&mut registers, &duc_path)?;
    let prefill = session.prefill_guarded_carrier()?;
    arm_guarded_tx(session.registers())?;

    let max_drive = tx_drive_watts_to_byte(VALIDATED_MAX_WATTS);
    let started = Instant::now();
    let mut next_drive_at = started;
    let mut drive = 0_u8;
    let mut peak_forward = 0.0_f32;
    let mut peak_reverse = 0.0_f32;
    let mut peak_swr = 1.0_f32;
    let mut peak_exciter_raw = 0_u16;
    let mut peak_forward_raw = 0_u16;
    let mut peak_reverse_raw = 0_u16;
    let mut fifo_lwm = prefill.occupied_words;
    let probe_result = (|| -> Result<PowerSample, XdmaError> {
        while started.elapsed() < config.duration {
            if TX_STOP_REQUESTED.load(Ordering::SeqCst) {
                return Err(XdmaError::Incompatible(
                    "guarded TX interrupted; receive-safe cleanup requested".into(),
                ));
            }
            let fifo = session.service_guarded_carrier()?;
            fifo_lwm = fifo_lwm.min(fifo.occupied_words);
            let power = read_power(session.registers(), config.power_meter_scale)?;
            peak_forward = peak_forward.max(power.forward_watts);
            peak_reverse = peak_reverse.max(power.reverse_watts);
            peak_swr = peak_swr.max(power.swr);
            peak_exciter_raw = peak_exciter_raw.max(power.exciter_raw);
            peak_forward_raw = peak_forward_raw.max(power.forward_raw);
            peak_reverse_raw = peak_reverse_raw.max(power.reverse_raw);
            enforce_power_limits(power)?;

            if Instant::now() >= next_drive_at
                && drive < max_drive
                && power.forward_watts < TX_TARGET_WATTS
            {
                drive += 1;
                set_drive_level(session.registers(), drive)?;
                next_drive_at += DRIVE_STEP_INTERVAL;
            }
            thread::sleep(TX_LOOP_SLEEP);
        }
        read_power(session.registers(), config.power_meter_scale)
    })();

    // Drop RF before any logging or validation can delay cleanup.
    let shutdown = shutdown_guarded_tx(session.registers());
    let stop = session.stop();
    drop(session);
    let close = registers.close_safely();
    let final_power = probe_result?;
    shutdown?;
    stop?;
    close?;
    if peak_forward < MIN_MEASURABLE_FORWARD_WATTS {
        return Err(XdmaError::Incompatible(format!(
            "guarded TX produced no measurable forward power: peak={:.3}W exciter_raw={} forward_raw={} reverse_raw={} final_drive={} fifo_lwm={}",
            peak_forward,
            peak_exciter_raw,
            peak_forward_raw,
            peak_reverse_raw,
            drive,
            fifo_lwm
        )));
    }
    record_probe_outcome(
        5,
        "guarded-tx",
        "passed",
        "rf-cleanup-verified",
        None,
        &[
            (
                "frequency_hz",
                TelemetryValue::number(VALIDATED_FREQUENCY_HZ),
            ),
            (
                "antenna",
                TelemetryValue::text(format!("ANT{VALIDATED_ANTENNA}")),
            ),
            (
                "duration_ms",
                TelemetryValue::number(config.duration.as_millis()),
            ),
            ("cpu", TelemetryValue::number(cpu)),
            ("scheduler", TelemetryValue::text(scheduler.to_string())),
            (
                "scheduler_priority",
                TelemetryValue::number(scheduler_priority),
            ),
            ("max_watts", TelemetryValue::number(VALIDATED_MAX_WATTS)),
            ("target_watts", TelemetryValue::number(TX_TARGET_WATTS)),
            ("max_drive", TelemetryValue::number(max_drive)),
            ("final_drive", TelemetryValue::number(drive)),
            (
                "final_exciter_raw",
                TelemetryValue::number(final_power.exciter_raw),
            ),
            (
                "final_forward_raw",
                TelemetryValue::number(final_power.forward_raw),
            ),
            (
                "final_reverse_raw",
                TelemetryValue::number(final_power.reverse_raw),
            ),
            ("peak_exciter_raw", TelemetryValue::number(peak_exciter_raw)),
            ("peak_forward_raw", TelemetryValue::number(peak_forward_raw)),
            ("peak_reverse_raw", TelemetryValue::number(peak_reverse_raw)),
            (
                "final_forward_watts",
                TelemetryValue::number(final_power.forward_watts),
            ),
            (
                "final_reverse_watts",
                TelemetryValue::number(final_power.reverse_watts),
            ),
            ("peak_forward_watts", TelemetryValue::number(peak_forward)),
            ("peak_reverse_watts", TelemetryValue::number(peak_reverse)),
            ("peak_swr", TelemetryValue::number(peak_swr)),
            ("fifo_lwm", TelemetryValue::number(fifo_lwm)),
            ("rf_cleanup", TelemetryValue::boolean(true)),
        ],
    );
    println!(
        "saturn-bridge: XDMA Phase 5 guarded TX probe passed frequency_hz={} antenna=ANT{} duration_ms={} cpu={} scheduler={} scheduler_priority={} max_watts={:.1} target_watts={:.1} max_drive={} final_drive={} exciter_raw={} forward_raw={} reverse_raw={} peak_exciter_raw={} peak_forward_raw={} peak_reverse_raw={} final_forward_watts={:.3} final_reverse_watts={:.3} peak_forward_watts={:.3} peak_reverse_watts={:.3} peak_swr={:.2} fifo_lwm={} rf_cleanup=verified",
        VALIDATED_FREQUENCY_HZ,
        VALIDATED_ANTENNA,
        config.duration.as_millis(),
        cpu,
        scheduler,
        scheduler_priority,
        VALIDATED_MAX_WATTS,
        TX_TARGET_WATTS,
        max_drive,
        drive,
        final_power.exciter_raw,
        final_power.forward_raw,
        final_power.reverse_raw,
        peak_exciter_raw,
        peak_forward_raw,
        peak_reverse_raw,
        final_power.forward_watts,
        final_power.reverse_watts,
        peak_forward,
        peak_reverse,
        peak_swr,
        fifo_lwm
    );
    Ok(())
}

fn register_path() -> PathBuf {
    env::var_os("SATURN_BRIDGE_XDMA_USER_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/xdma0_user"))
}

fn verify_completion_policy() -> Result<(), XdmaError> {
    let path = "/sys/module/xdma/parameters/completion_kthread_priority";
    let value = std::fs::read_to_string(path).map_err(|source| XdmaError::Io {
        action: "could not read XDMA completion-thread priority",
        source,
    })?;
    let priority = value
        .trim()
        .parse::<i32>()
        .map_err(|_| XdmaError::Incompatible(format!("{path} does not contain an integer")))?;
    if priority != REQUIRED_COMPLETION_PRIORITY {
        return Err(XdmaError::Incompatible(format!(
            "guarded TX requires XDMA completion_kthread_priority={REQUIRED_COMPLETION_PRIORITY}, found {priority}"
        )));
    }
    Ok(())
}

fn configure_safe_tx_baseline(registers: &XdmaRegisterDevice) -> Result<(), XdmaError> {
    let safe_dac = dac_control_word(0);
    let phase_word = frequency_to_phase_word(VALIDATED_FREQUENCY_HZ);
    registers.write_register(DAC_CONTROL_REGISTER, safe_dac)?;
    registers.write_register(TX_DUC_REGISTER, phase_word)?;
    let safe_alex = ALEX_60_40M_LPF_BIT | ALEX_ANT1_BIT;
    registers.write_register(ALEX_TX_FILTER_REGISTER, u32::from(ALEX_60_40M_LPF_BIT))?;
    registers.write_register(ALEX_TX_ANTENNA_REGISTER, u32::from(safe_alex))?;
    registers.update_register(
        RF_GPIO_REGISTER,
        |value| value | RF_DATA_NETWORK_ENDIAN_BIT,
        "could not select proven P2 DUC byte order",
    )?;
    registers.verify_safe_receive_state()?;
    let actual_dac = registers.read_register(DAC_CONTROL_REGISTER)?;
    let actual_phase = registers.read_register(TX_DUC_REGISTER)?;
    let actual_filter = registers.read_register(ALEX_TX_FILTER_REGISTER)?;
    let actual_antenna = registers.read_register(ALEX_TX_ANTENNA_REGISTER)?;
    let actual_gpio = registers.read_register(RF_GPIO_REGISTER)?;
    if actual_dac != safe_dac
        || actual_phase != phase_word
        || actual_filter != u32::from(ALEX_60_40M_LPF_BIT)
        || actual_antenna != u32::from(safe_alex)
        || actual_gpio & RF_DATA_NETWORK_ENDIAN_BIT == 0
    {
        return Err(XdmaError::Incompatible(format!(
            "guarded TX baseline readback failed: dac=0x{actual_dac:08x} phase=0x{actual_phase:08x} filter=0x{actual_filter:08x} antenna=0x{actual_antenna:08x} gpio=0x{actual_gpio:08x}"
        )));
    }
    Ok(())
}

fn arm_guarded_tx(registers: &XdmaRegisterDevice) -> Result<(), XdmaError> {
    registers.update_register(
        TX_CONFIG_REGISTER,
        guarded_tx_config,
        "could not arm guarded direct-XDMA TX configuration",
    )?;
    let keyed_alex = ALEX_60_40M_LPF_BIT | ALEX_ANT1_BIT | ALEX_TX_RELAY_BIT;
    registers.write_register(ALEX_TX_FILTER_REGISTER, u32::from(ALEX_60_40M_LPF_BIT))?;
    registers.write_register(ALEX_TX_ANTENNA_REGISTER, u32::from(keyed_alex))?;
    registers.update_register(
        RF_GPIO_REGISTER,
        |value| (value | TX_ENABLE_BIT) & !(MOX_BIT | TX_RELAY_DISABLE_BIT),
        "could not enable guarded TX hardware",
    )?;
    registers.update_register(
        RF_GPIO_REGISTER,
        |value| value | MOX_BIT,
        "could not assert guarded TX MOX",
    )?;
    verify_guarded_tx_state(registers)
}

fn shutdown_guarded_tx(registers: &XdmaRegisterDevice) -> Result<(), XdmaError> {
    // Attempt every independent shutdown action even when an earlier register
    // operation fails. Power-producing controls are cleared before relay or
    // filter work, but no error can skip the later MOX/TX-enable clear.
    let safe_dac = dac_control_word(0);
    let drive = registers.write_register(DAC_CONTROL_REGISTER, safe_dac);
    let amplitude = registers.update_register(
        TX_CONFIG_REGISTER,
        |value| value & !TX_AMPLITUDE_MASK,
        "could not zero guarded TX amplitude",
    );
    let gpio = registers.update_register(
        RF_GPIO_REGISTER,
        |value| (value & !(MOX_BIT | TX_ENABLE_BIT)) | TX_RELAY_DISABLE_BIT,
        "could not return guarded TX GPIO to receive",
    );
    let safe_alex = ALEX_60_40M_LPF_BIT | ALEX_ANT1_BIT;
    let alex = registers.write_register(ALEX_TX_ANTENNA_REGISTER, u32::from(safe_alex));
    let verify = (|| {
        let actual_dac = registers.read_register(DAC_CONTROL_REGISTER)?;
        let actual_gpio = registers.read_register(RF_GPIO_REGISTER)?;
        let actual_tx = registers.read_register(TX_CONFIG_REGISTER)?;
        let actual_alex = registers.read_register(ALEX_TX_ANTENNA_REGISTER)?;
        if actual_dac != safe_dac
            || actual_gpio & (MOX_BIT | TX_ENABLE_BIT) != 0
            || actual_gpio & TX_RELAY_DISABLE_BIT == 0
            || actual_tx & TX_AMPLITUDE_MASK != 0
            || actual_alex != u32::from(safe_alex)
        {
            return Err(XdmaError::Incompatible(format!(
                "guarded TX shutdown readback failed: dac=0x{actual_dac:08x} gpio=0x{actual_gpio:08x} tx=0x{actual_tx:08x} alex=0x{actual_alex:08x}"
            )));
        }
        Ok(())
    })();
    drive.and(amplitude).and(gpio).and(alex).and(verify)
}

fn guarded_tx_config(current: u32) -> u32 {
    (current
        & !(TX_MODULATION_SOURCE_MASK
            | TX_OUTPUT_GATE_BIT
            | TX_AMPLITUDE_MASK
            | TX_WATCHDOG_OVERRIDE_BIT
            | DUC_MUX_RESET_BIT
            | TX_IQ_DEINTERLEAVE_BIT
            | DUC_STREAM_ENABLE_BIT))
        | TX_PROTOCOL_P2_BIT
        | (PCB2_FW13_TX_AMPLITUDE << 4)
        | DUC_STREAM_ENABLE_BIT
}

fn verify_guarded_tx_state(registers: &XdmaRegisterDevice) -> Result<(), XdmaError> {
    let gpio = registers.read_register(RF_GPIO_REGISTER)?;
    let tx = registers.read_register(TX_CONFIG_REGISTER)?;
    let alex = registers.read_register(ALEX_TX_ANTENNA_REGISTER)?;
    let expected_tx = TX_PROTOCOL_P2_BIT | (PCB2_FW13_TX_AMPLITUDE << 4) | DUC_STREAM_ENABLE_BIT;
    let relevant_tx = TX_MODULATION_SOURCE_MASK
        | TX_OUTPUT_GATE_BIT
        | TX_PROTOCOL_P2_BIT
        | TX_AMPLITUDE_MASK
        | TX_WATCHDOG_OVERRIDE_BIT
        | DUC_MUX_RESET_BIT
        | TX_IQ_DEINTERLEAVE_BIT
        | DUC_STREAM_ENABLE_BIT;
    if gpio & (MOX_BIT | TX_ENABLE_BIT) != MOX_BIT | TX_ENABLE_BIT
        || gpio & RF_DATA_NETWORK_ENDIAN_BIT == 0
        || gpio & TX_RELAY_DISABLE_BIT != 0
        || tx & relevant_tx != expected_tx
        || alex != u32::from(ALEX_60_40M_LPF_BIT | ALEX_ANT1_BIT | ALEX_TX_RELAY_BIT)
    {
        return Err(XdmaError::Incompatible(format!(
            "guarded TX readback failed: gpio=0x{gpio:08x} tx=0x{tx:08x} alex=0x{alex:08x}"
        )));
    }
    Ok(())
}

fn read_power(
    registers: &XdmaRegisterDevice,
    power_meter_scale: f32,
) -> Result<PowerSample, XdmaError> {
    let exciter_raw = registers
        .read_register(ALEX_EXCITER_POWER_REGISTER)?
        .min(u32::from(u16::MAX)) as u16;
    let forward_raw = registers
        .read_register(ALEX_FORWARD_POWER_REGISTER)?
        .min(u32::from(u16::MAX)) as u16;
    let reverse_raw = registers
        .read_register(ALEX_REVERSE_POWER_REGISTER)?
        .min(u32::from(u16::MAX)) as u16;
    let forward_watts = saturn_adc_to_watts(forward_raw, 32, power_meter_scale);
    let reverse_watts = saturn_adc_to_watts(reverse_raw, 28, power_meter_scale);
    Ok(PowerSample {
        exciter_raw,
        forward_raw,
        reverse_raw,
        forward_watts,
        reverse_watts,
        swr: calculate_swr(forward_watts, reverse_watts),
    })
}

fn enforce_power_limits(power: PowerSample) -> Result<(), XdmaError> {
    if power.forward_watts > VALIDATED_MAX_WATTS {
        return Err(XdmaError::Incompatible(format!(
            "guarded TX forward-power trip: {:.3} W exceeds {:.1} W",
            power.forward_watts, VALIDATED_MAX_WATTS
        )));
    }
    if power.reverse_watts > REVERSE_POWER_TRIP_WATTS {
        return Err(XdmaError::Incompatible(format!(
            "guarded TX reverse-power trip: {:.3} W exceeds {:.2} W",
            power.reverse_watts, REVERSE_POWER_TRIP_WATTS
        )));
    }
    if power.forward_watts >= SWR_MIN_FORWARD_WATTS && power.swr > SWR_TRIP {
        return Err(XdmaError::Incompatible(format!(
            "guarded TX SWR trip: {:.2} exceeds {:.1}",
            power.swr, SWR_TRIP
        )));
    }
    Ok(())
}

fn frequency_to_phase_word(frequency_hz: u32) -> u32 {
    let numerator = u128::from(frequency_hz) * (1u128 << 32);
    ((numerator + 61_440_000) / 122_880_000) as u32
}

fn tx_drive_watts_to_byte(watts: f32) -> u8 {
    let watts = watts.clamp(0.0, 100.0);
    if watts == 0.0 {
        return 0;
    }
    let curve = [
        (0.0_f32, 0.0_f32),
        (5.0, 18.0),
        (10.0, 24.0),
        (25.0, 34.0),
        (50.0, 46.0),
        (100.0, 68.0),
    ];
    let mut lower = curve[0];
    let mut upper = curve[curve.len() - 1];
    for window in curve.windows(2) {
        if watts <= window[1].0 {
            lower = window[0];
            upper = window[1];
            break;
        }
    }
    let fraction = (watts - lower.0) / (upper.0 - lower.0);
    (lower.1 + (upper.1 - lower.1) * fraction)
        .round()
        .clamp(1.0, 68.0) as u8
}

fn dac_control_word(level: u8) -> u32 {
    if level == 0 {
        return 0x3f3f_0000;
    }
    let desired_atten = 20.0 * (255.0_f64 / f64::from(level)).log10();
    let step = (2.0 * desired_atten).floor().clamp(0.0, 63.0) as u32;
    let residual_atten = desired_atten - f64::from(step) * 0.5;
    let dac = (255.0 / 10.0_f64.powf(residual_atten / 20.0))
        .floor()
        .clamp(0.0, 255.0) as u32;
    dac | (dac << 8) | (step << 16) | (step << 24)
}

fn set_drive_level(registers: &XdmaRegisterDevice, level: u8) -> Result<(), XdmaError> {
    let expected = dac_control_word(level);
    registers.write_register(DAC_CONTROL_REGISTER, expected)?;
    let actual = registers.read_register(DAC_CONTROL_REGISTER)?;
    if actual != expected {
        return Err(XdmaError::Incompatible(format!(
            "guarded TX drive readback failed at level {level}: expected=0x{expected:08x} actual=0x{actual:08x}"
        )));
    }
    Ok(())
}

fn saturn_adc_to_watts(raw: u16, offset: i32, scale: f32) -> f32 {
    let corrected = (i32::from(raw) - offset).max(0) as f32;
    let volts = corrected / 4095.0 * 5.0;
    (volts * volts / 0.12) * scale
}

fn calculate_swr(forward_watts: f32, reverse_watts: f32) -> f32 {
    if forward_watts <= 0.0 || reverse_watts <= 0.0 {
        return 1.0;
    }
    if reverse_watts >= forward_watts {
        return 99.0;
    }
    let ratio = (reverse_watts / forward_watts).sqrt();
    ((1.0 + ratio) / (1.0 - ratio)).clamp(1.0, 99.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_injection_is_default_off_and_bounded() {
        assert_eq!(
            parse_failure_injection(None).unwrap(),
            FailureInjection::None
        );
        assert_eq!(
            parse_failure_injection(Some("after-open")).unwrap(),
            FailureInjection::AfterOpen
        );
        assert_eq!(
            parse_failure_injection(Some("after-verify")).unwrap(),
            FailureInjection::AfterVerify
        );
        assert!(parse_failure_injection(Some("after-key")).is_err());
        assert!(parse_failure_injection(Some("")).is_err());
    }

    #[test]
    fn validated_frequency_and_drive_match_existing_p2_path() {
        assert_eq!(frequency_to_phase_word(30_720_000), 0x4000_0000);
        assert_eq!(tx_drive_watts_to_byte(0.0), 0);
        assert_eq!(tx_drive_watts_to_byte(1.0), 4);
        assert_eq!(tx_drive_watts_to_byte(3.0), 11);
        assert_eq!(tx_drive_watts_to_byte(5.0), 18);
        assert_eq!(tx_drive_watts_to_byte(100.0), 68);
    }

    #[test]
    fn dac_drive_zero_is_maximum_attenuation() {
        assert_eq!(dac_control_word(0), 0x3f3f_0000);
        assert_eq!(dac_control_word(255), 0x0000_ffff);
        assert_ne!(dac_control_word(11), dac_control_word(0));
    }

    #[test]
    fn guarded_config_keeps_watchdog_and_always_on_gate_disabled() {
        let configured = guarded_tx_config(u32::MAX);
        assert_eq!(configured & TX_OUTPUT_GATE_BIT, 0);
        assert_eq!(configured & TX_WATCHDOG_OVERRIDE_BIT, 0);
        assert_eq!(configured & TX_MODULATION_SOURCE_MASK, 0);
        assert_eq!(configured & TX_IQ_DEINTERLEAVE_BIT, 0);
        assert_eq!(configured & TX_AMPLITUDE_MASK, PCB2_FW13_TX_AMPLITUDE << 4);
        assert_ne!(configured & DUC_STREAM_ENABLE_BIT, 0);
        assert_ne!(configured & TX_PROTOCOL_P2_BIT, 0);
    }

    #[test]
    fn power_and_swr_guards_are_conservative() {
        assert!((saturn_adc_to_watts(32, 32, 1.0) - 0.0).abs() < f32::EPSILON);
        assert_eq!(calculate_swr(1.0, 0.0), 1.0);
        assert_eq!(calculate_swr(1.0, 1.0), 99.0);
        assert!(calculate_swr(1.0, 0.25) >= 3.0);
        assert!(enforce_power_limits(PowerSample {
            exciter_raw: 0,
            forward_raw: 0,
            reverse_raw: 0,
            forward_watts: 3.01,
            reverse_watts: 0.0,
            swr: 1.0,
        })
        .is_err());
    }
}
