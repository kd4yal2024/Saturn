//! Offline-testable host components. Production main does not use the new
//! PureSignal controller until the native and RXC1 evidence gates are resolved.
pub mod diagnostic_fft;
pub mod puresignal;
pub mod rf_spectrum;
pub mod signal_diagnostics;
