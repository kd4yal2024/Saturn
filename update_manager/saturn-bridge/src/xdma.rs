//! Phase 1 of the direct Saturn/XDMA backend.
//!
//! This module deliberately does not open any DMA stream device.  It provides
//! the hardware-identity, compatibility, ownership, register-access, and
//! fail-safe shutdown foundation needed before an RX data path is added.

use crate::xdma_telemetry::{record_probe_outcome, TelemetryValue};
use std::env;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_USER_DEVICE: &str = "/dev/xdma0_user";

const USER_VERSION_REGISTER: u64 = 0x4004;
const SOFTWARE_VERSION_REGISTER: u64 = 0xC000;
const PRODUCT_VERSION_REGISTER: u64 = 0xC004;
const KEYER_CONFIG_REGISTER: u64 = 0x2000;
const TX_CONFIG_REGISTER: u64 = 0x2008;
const RF_GPIO_REGISTER: u64 = 0x2014;

const SATURN_PRODUCT_ID: u16 = 1;
const GOLDEN_SOFTWARE_ID: u8 = 3;
const PRIMARY_SOFTWARE_ID: u8 = 4;
const REQUIRED_FIRMWARE_MAJOR: u8 = 1;
const REQUIRED_CLOCK_MASK: u8 = 0x0F;

const MOX_BIT: u32 = 1 << 24;
const TX_ENABLE_BIT: u32 = 1 << 25;
const TX_RELAY_DISABLE_BIT: u32 = 1 << 27;
const CW_KEYER_ENABLE_BIT: u32 = 1 << 31;
const TX_WATCHDOG_OVERRIDE_BIT: u32 = 1 << 28;
const TX_MODULATION_SOURCE_MASK: u32 = 0b11;
const TX_OUTPUT_GATE_BIT: u32 = 1 << 2;
const TX_AMPLITUDE_MASK: u32 = 0x3ffff << 4;
const DUC_MUX_RESET_BIT: u32 = 1 << 29;
const TX_IQ_DEINTERLEAVE_BIT: u32 = 1 << 30;
const DUC_STREAM_ENABLE_BIT: u32 = 1 << 31;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaturnIdentity {
    pub product_id: u16,
    pub pcb_version: u16,
    pub software_id: u8,
    pub firmware_major: u8,
    pub firmware_minor: u16,
    pub clock_mask: u8,
    pub user_version: u32,
}

impl SaturnIdentity {
    fn decode(software: u32, product: u32, user_version: u32) -> Self {
        Self {
            product_id: (product >> 16) as u16,
            pcb_version: product as u16,
            software_id: ((software >> 20) & 0x1F) as u8,
            firmware_major: ((software >> 25) & 0x7F) as u8,
            firmware_minor: ((software >> 4) & 0xFFFF) as u16,
            clock_mask: (software & 0x0F) as u8,
            user_version,
        }
    }

    pub fn is_fallback(&self) -> bool {
        self.software_id == GOLDEN_SOFTWARE_ID
    }

    fn validate(&self) -> Result<(), XdmaError> {
        if self.product_id != SATURN_PRODUCT_ID {
            return Err(XdmaError::Incompatible(format!(
                "XDMA product ID {} is not Saturn product ID {}",
                self.product_id, SATURN_PRODUCT_ID
            )));
        }
        if !matches!(self.software_id, GOLDEN_SOFTWARE_ID | PRIMARY_SOFTWARE_ID) {
            return Err(XdmaError::Incompatible(format!(
                "Saturn software ID {} is neither golden ({}) nor primary ({})",
                self.software_id, GOLDEN_SOFTWARE_ID, PRIMARY_SOFTWARE_ID
            )));
        }
        if self.clock_mask != REQUIRED_CLOCK_MASK {
            return Err(XdmaError::Incompatible(format!(
                "Saturn clock mask is 0x{:02x}; expected 0x{:02x}",
                self.clock_mask, REQUIRED_CLOCK_MASK
            )));
        }
        if self.firmware_major != REQUIRED_FIRMWARE_MAJOR {
            return Err(XdmaError::Incompatible(format!(
                "Saturn FPGA firmware major {} is incompatible; expected {}",
                self.firmware_major, REQUIRED_FIRMWARE_MAJOR
            )));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum XdmaError {
    Io {
        action: &'static str,
        source: io::Error,
    },
    Incompatible(String),
    Ownership(String),
}

impl fmt::Display for XdmaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { action, source } => write!(f, "{action}: {source}"),
            Self::Incompatible(message) | Self::Ownership(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for XdmaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Incompatible(_) | Self::Ownership(_) => None,
        }
    }
}

pub struct XdmaRegisterDevice {
    file: File,
    path: PathBuf,
    identity: SaturnIdentity,
    armed_for_cleanup: bool,
}

impl XdmaRegisterDevice {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, XdmaError> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|source| XdmaError::Io {
                action: "could not open XDMA register device",
                source,
            })?;
        let software = read_u32(&file, SOFTWARE_VERSION_REGISTER)?;
        let product = read_u32(&file, PRODUCT_VERSION_REGISTER)?;
        let user_version = read_u32(&file, USER_VERSION_REGISTER)?;
        let identity = SaturnIdentity::decode(software, product, user_version);
        identity.validate()?;

        let mut device = Self {
            file,
            path,
            identity,
            armed_for_cleanup: true,
        };
        device.force_safe_receive_state()?;
        Ok(device)
    }

    pub fn identity(&self) -> &SaturnIdentity {
        &self.identity
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn read_register(&self, offset: u64) -> Result<u32, XdmaError> {
        read_u32(&self.file, offset)
    }

    pub(crate) fn write_register(&self, offset: u64, value: u32) -> Result<(), XdmaError> {
        write_u32(&self.file, offset, value)
    }

    pub(crate) fn update_register(
        &self,
        offset: u64,
        update: impl FnOnce(u32) -> u32,
        action: &'static str,
    ) -> Result<(), XdmaError> {
        update_register(&self.file, offset, update, action)
    }

    /// Force the minimum safe, non-transmitting hardware state.
    ///
    /// Register values are read-modify-written so unrelated GPIO, keyer, and
    /// TX configuration bits are preserved.
    pub fn force_safe_receive_state(&mut self) -> Result<(), XdmaError> {
        update_register(
            &self.file,
            RF_GPIO_REGISTER,
            |value| (value & !(MOX_BIT | TX_ENABLE_BIT)) | TX_RELAY_DISABLE_BIT,
            "could not force Saturn RF GPIO into receive state",
        )?;
        update_register(
            &self.file,
            KEYER_CONFIG_REGISTER,
            |value| value & !CW_KEYER_ENABLE_BIT,
            "could not disable Saturn CW keyer",
        )?;
        update_register(
            &self.file,
            TX_CONFIG_REGISTER,
            |value| {
                value
                    & !(TX_MODULATION_SOURCE_MASK
                        | TX_OUTPUT_GATE_BIT
                        | TX_AMPLITUDE_MASK
                        | TX_WATCHDOG_OVERRIDE_BIT
                        | DUC_MUX_RESET_BIT
                        | TX_IQ_DEINTERLEAVE_BIT
                        | DUC_STREAM_ENABLE_BIT)
            },
            "could not disable Saturn transmit data path",
        )?;
        self.verify_safe_receive_state()
    }

    pub(crate) fn verify_safe_receive_state(&self) -> Result<(), XdmaError> {
        let gpio = self.read_register(RF_GPIO_REGISTER)?;
        let keyer = self.read_register(KEYER_CONFIG_REGISTER)?;
        let tx = self.read_register(TX_CONFIG_REGISTER)?;
        let unsafe_tx = TX_MODULATION_SOURCE_MASK
            | TX_OUTPUT_GATE_BIT
            | TX_AMPLITUDE_MASK
            | TX_WATCHDOG_OVERRIDE_BIT
            | DUC_MUX_RESET_BIT
            | TX_IQ_DEINTERLEAVE_BIT
            | DUC_STREAM_ENABLE_BIT;
        if gpio & (MOX_BIT | TX_ENABLE_BIT) != 0
            || gpio & TX_RELAY_DISABLE_BIT == 0
            || keyer & CW_KEYER_ENABLE_BIT != 0
            || tx & unsafe_tx != 0
        {
            return Err(XdmaError::Incompatible(format!(
                "Saturn receive-safe readback failed: gpio=0x{gpio:08x} keyer=0x{keyer:08x} tx=0x{tx:08x}"
            )));
        }
        Ok(())
    }

    pub fn close_safely(mut self) -> Result<(), XdmaError> {
        self.force_safe_receive_state()?;
        self.armed_for_cleanup = false;
        Ok(())
    }
}

impl Drop for XdmaRegisterDevice {
    fn drop(&mut self) {
        if self.armed_for_cleanup {
            if let Err(error) = self.force_safe_receive_state() {
                eprintln!("saturn-bridge: XDMA emergency receive-state cleanup failed: {error}");
            }
        }
    }
}

pub fn run_phase1_probe() -> Result<(), XdmaError> {
    ensure_p2app_inactive()?;
    let path = env::var_os("SATURN_BRIDGE_XDMA_USER_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_USER_DEVICE));
    let device = XdmaRegisterDevice::open(&path)?;
    let identity = device.identity().clone();
    let device_path = device.path().display().to_string();
    let image = if identity.is_fallback() {
        "golden/fallback"
    } else {
        "primary"
    };
    println!(
        "saturn-bridge: XDMA Phase 1 probe passed device={} product={} pcb={} firmware={}.{} software_id={} image={} clocks=0x{:02x} user_version=0x{:08x}",
        device_path,
        identity.product_id,
        identity.pcb_version,
        identity.firmware_major,
        identity.firmware_minor,
        identity.software_id,
        image,
        identity.clock_mask,
        identity.user_version
    );
    device.close_safely()?;
    record_probe_outcome(
        1,
        "identity",
        "passed",
        "receive-safe-verified",
        None,
        &[
            ("device", TelemetryValue::text(device_path)),
            ("product", TelemetryValue::number(identity.product_id)),
            ("pcb", TelemetryValue::number(identity.pcb_version)),
            (
                "firmware",
                TelemetryValue::text(format!(
                    "{}.{}",
                    identity.firmware_major, identity.firmware_minor
                )),
            ),
            ("software_id", TelemetryValue::number(identity.software_id)),
            ("image", TelemetryValue::text(image)),
            ("clock_mask", TelemetryValue::number(identity.clock_mask)),
            (
                "user_version",
                TelemetryValue::number(identity.user_version),
            ),
            ("rf_keyed", TelemetryValue::boolean(false)),
        ],
    );
    println!(
        "saturn-bridge: XDMA Phase 1 probe completed; MOX, TX enable, PA relay, CW keyer, and DUC stream are safely disabled"
    );
    Ok(())
}

pub(crate) fn ensure_p2app_inactive() -> Result<(), XdmaError> {
    let status = match Command::new("systemctl")
        .args(["is-active", "--quiet", "p2app.service"])
        .status()
    {
        Ok(status) => status,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(XdmaError::Ownership(
                "cannot verify exclusive XDMA ownership because systemctl is unavailable".into(),
            ));
        }
        Err(source) => {
            return Err(XdmaError::Io {
                action: "could not check p2app.service ownership",
                source,
            });
        }
    };
    classify_p2app_status(status.code())
}

fn classify_p2app_status(status_code: Option<i32>) -> Result<(), XdmaError> {
    match status_code {
        Some(0) => Err(XdmaError::Ownership(
            "p2app.service is active; stop it before running the direct XDMA probe".into(),
        )),
        Some(3 | 4) => Ok(()),
        Some(code) => Err(XdmaError::Ownership(format!(
            "cannot verify exclusive XDMA ownership because systemctl exited with status {code}"
        ))),
        None => Err(XdmaError::Ownership(
            "cannot verify exclusive XDMA ownership because systemctl was terminated".into(),
        )),
    }
}

fn read_u32(file: &File, offset: u64) -> Result<u32, XdmaError> {
    let mut bytes = [0u8; 4];
    file.read_exact_at(&mut bytes, offset)
        .map_err(|source| XdmaError::Io {
            action: "could not read XDMA register",
            source,
        })?;
    Ok(u32::from_le_bytes(bytes))
}

fn write_u32(file: &File, offset: u64, value: u32) -> Result<(), XdmaError> {
    file.write_all_at(&value.to_le_bytes(), offset)
        .map_err(|source| XdmaError::Io {
            action: "could not write XDMA register",
            source,
        })
}

fn update_register(
    file: &File,
    offset: u64,
    update: impl FnOnce(u32) -> u32,
    action: &'static str,
) -> Result<(), XdmaError> {
    let current = read_u32(file, offset)?;
    let next = update(current);
    if next != current {
        write_u32(file, offset, next).map_err(|error| match error {
            XdmaError::Io { source, .. } => XdmaError::Io { action, source },
            other => other,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Fixture {
        path: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "saturn-bridge-xdma-test-{}-{nonce}",
                std::process::id()
            ));
            let file = File::create(&path).unwrap();
            file.set_len(PRODUCT_VERSION_REGISTER + 4).unwrap();
            Self { path }
        }

        fn file(&self) -> File {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.path)
                .unwrap()
        }

        fn write(&self, offset: u64, value: u32) {
            write_u32(&self.file(), offset, value).unwrap();
        }

        fn read(&self, offset: u64) -> u32 {
            read_u32(&self.file(), offset).unwrap()
        }

        fn install_valid_identity(&self) {
            let software = (u32::from(REQUIRED_FIRMWARE_MAJOR) << 25)
                | (u32::from(PRIMARY_SOFTWARE_ID) << 20)
                | (18u32 << 4)
                | u32::from(REQUIRED_CLOCK_MASK);
            self.write(SOFTWARE_VERSION_REGISTER, software);
            self.write(
                PRODUCT_VERSION_REGISTER,
                (u32::from(SATURN_PRODUCT_ID) << 16) | 3,
            );
            self.write(USER_VERSION_REGISTER, 0x2026_0729);
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn decodes_and_validates_primary_saturn_identity() {
        let software = (1 << 25) | (4 << 20) | (19 << 4) | 0x0F;
        let identity = SaturnIdentity::decode(software, (1 << 16) | 3, 0x1234_5678);
        assert_eq!(identity.product_id, 1);
        assert_eq!(identity.pcb_version, 3);
        assert_eq!(identity.software_id, 4);
        assert_eq!(identity.firmware_major, 1);
        assert_eq!(identity.firmware_minor, 19);
        assert_eq!(identity.clock_mask, 0x0F);
        assert!(!identity.is_fallback());
        identity.validate().unwrap();
    }

    #[test]
    fn rejects_non_saturn_and_missing_clocks() {
        let not_saturn = SaturnIdentity::decode((1 << 25) | (4 << 20) | 0x0F, 2 << 16, 0);
        assert!(not_saturn.validate().is_err());
        let missing_clock = SaturnIdentity::decode((1 << 25) | (4 << 20) | 0x07, 1 << 16, 0);
        assert!(missing_clock.validate().is_err());
    }

    #[test]
    fn open_forces_safe_state_without_clobbering_unrelated_bits() {
        let fixture = Fixture::new();
        fixture.install_valid_identity();
        let gpio_unrelated = 0x0000_0055;
        fixture.write(RF_GPIO_REGISTER, gpio_unrelated | MOX_BIT | TX_ENABLE_BIT);
        fixture.write(KEYER_CONFIG_REGISTER, 0x0000_0033 | CW_KEYER_ENABLE_BIT);
        fixture.write(
            TX_CONFIG_REGISTER,
            0x0000_0008
                | TX_MODULATION_SOURCE_MASK
                | TX_OUTPUT_GATE_BIT
                | TX_AMPLITUDE_MASK
                | TX_WATCHDOG_OVERRIDE_BIT
                | DUC_MUX_RESET_BIT
                | TX_IQ_DEINTERLEAVE_BIT
                | DUC_STREAM_ENABLE_BIT,
        );

        let device = XdmaRegisterDevice::open(&fixture.path).unwrap();
        assert_eq!(
            fixture.read(RF_GPIO_REGISTER),
            gpio_unrelated | TX_RELAY_DISABLE_BIT
        );
        assert_eq!(fixture.read(KEYER_CONFIG_REGISTER), 0x0000_0033);
        assert_eq!(fixture.read(TX_CONFIG_REGISTER), 0x0000_0008);
        device.close_safely().unwrap();
    }

    #[test]
    fn drop_recovers_unsafe_registers_after_an_injected_failure() {
        let fixture = Fixture::new();
        fixture.install_valid_identity();
        let device = XdmaRegisterDevice::open(&fixture.path).unwrap();
        device
            .write_register(RF_GPIO_REGISTER, MOX_BIT | TX_ENABLE_BIT | 0x0000_0055)
            .unwrap();
        device
            .write_register(KEYER_CONFIG_REGISTER, CW_KEYER_ENABLE_BIT | 0x33)
            .unwrap();
        device
            .write_register(
                TX_CONFIG_REGISTER,
                TX_MODULATION_SOURCE_MASK
                    | TX_OUTPUT_GATE_BIT
                    | TX_AMPLITUDE_MASK
                    | TX_WATCHDOG_OVERRIDE_BIT
                    | DUC_MUX_RESET_BIT
                    | TX_IQ_DEINTERLEAVE_BIT
                    | DUC_STREAM_ENABLE_BIT,
            )
            .unwrap();

        drop(device);

        assert_eq!(
            fixture.read(RF_GPIO_REGISTER),
            TX_RELAY_DISABLE_BIT | 0x0000_0055
        );
        assert_eq!(fixture.read(KEYER_CONFIG_REGISTER), 0x33);
        assert_eq!(fixture.read(TX_CONFIG_REGISTER), 0);
    }

    #[test]
    fn incompatible_identity_is_rejected_before_register_writes() {
        let fixture = Fixture::new();
        fixture.install_valid_identity();
        fixture.write(PRODUCT_VERSION_REGISTER, (2 << 16) | 3);
        let original_gpio = MOX_BIT | TX_ENABLE_BIT | 0x25;
        fixture.write(RF_GPIO_REGISTER, original_gpio);

        let error = match XdmaRegisterDevice::open(&fixture.path) {
            Ok(_) => panic!("non-Saturn identity unexpectedly accepted"),
            Err(error) => error,
        };
        assert!(matches!(error, XdmaError::Incompatible(_)));
        assert_eq!(fixture.read(RF_GPIO_REGISTER), original_gpio);
    }

    #[test]
    fn p2app_status_check_fails_closed() {
        assert!(classify_p2app_status(Some(3)).is_ok());
        assert!(classify_p2app_status(Some(4)).is_ok());
        assert!(matches!(
            classify_p2app_status(Some(0)),
            Err(XdmaError::Ownership(_))
        ));
        assert!(matches!(
            classify_p2app_status(Some(1)),
            Err(XdmaError::Ownership(_))
        ));
        assert!(matches!(
            classify_p2app_status(None),
            Err(XdmaError::Ownership(_))
        ));
    }
}
