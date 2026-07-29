//! Phase 5 guarded-transmit preflight.
//!
//! This slice does not key RF. It proves exclusive ownership, compatible
//! hardware identity, receive-safe register readback, and emergency cleanup
//! before a later explicitly authorized low-power transmit probe is added.

use crate::xdma::{ensure_p2app_inactive, XdmaError, XdmaRegisterDevice};
use std::env;
use std::path::PathBuf;

const FAILURE_INJECTION_ENV: &str = "SATURN_BRIDGE_XDMA_TX_PREFLIGHT_INJECT_FAILURE";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailureInjection {
    None,
    AfterOpen,
    AfterVerify,
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

pub fn run_phase5_tx_preflight() -> Result<(), XdmaError> {
    ensure_p2app_inactive()?;
    let injection = parse_failure_injection(env::var(FAILURE_INJECTION_ENV).ok().as_deref())?;
    let register_path = env::var_os("SATURN_BRIDGE_XDMA_USER_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/xdma0_user"));

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
}
