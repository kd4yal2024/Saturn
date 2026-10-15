//! Read-only identity query: called before FFTW wisdom, sockets or devices.
//! Version/source labels are not RF/topology qualification.
#[cfg(not(saturn_bridge_stub_native))]
unsafe extern "C" {
    fn GetWDSPVersion() -> i32;
}

pub(crate) fn print() {
    let wiring = if cfg!(feature = "managed-puresignal") {
        "opt-in-review"
    } else {
        "not-built"
    };
    #[cfg(not(saturn_bridge_stub_native))]
    let version = unsafe { GetWDSPVersion() };
    #[cfg(saturn_bridge_stub_native)]
    let version = 0;
    println!("{{\"format\":\"saturn-native-build-info-v1\",\"target\":\"{}\",\"sourcePin\":\"{}\",\"wdspVersion\":{},\"wdspArchiveSha256\":\"{}\",\"wdspHeaderSha256\":\"{}\",\"stubNative\":{},\"defaultTxOwner\":\"legacy\",\"productionManagedWiring\":\"{}\",\"rfQualified\":false}}",
        std::env::consts::ARCH, env!("SATURN_BRIDGE_SOURCE_PIN"), version,
        env!("SATURN_BRIDGE_WDSP_ARCHIVE_SHA256"), env!("SATURN_BRIDGE_WDSP_HEADER_SHA256"),
        cfg!(saturn_bridge_stub_native),wiring);
}
