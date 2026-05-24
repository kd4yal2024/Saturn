use std::env;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use crate::p2::ports::{P2PortMap, COMMAND_DISCOVERY_PORT};

#[derive(Clone, Debug)]
pub struct BridgeConfig {
    pub radio_command_addr: SocketAddr,
    pub client_bind_addr: SocketAddr,
    pub tci_bind_addr: SocketAddr,
    pub port_map: P2PortMap,
    pub enable_discovery: bool,
    pub discovery_timeout: Duration,
    pub receive_timeout: Duration,
    pub high_priority_period: Duration,
    pub rx_ddc_index: u8,
    pub ddc0_frequency_hz: u32,
    pub ddc0_adc: u8,
    pub ddc0_sample_rate_khz: u16,
    pub max_client_ddc0_sample_rate_khz: u16,
    pub ddc0_sample_size_bits: u8,
    pub rx_fft_size: u32,
    pub rx_low_latency: bool,
    pub tx_fft_size: u32,
    pub tx_low_latency: bool,
    pub remote_tx_100w_drive_byte: u8,
    pub tx_power_meter_scale: f32,
    pub remote_tx_rf_enabled: bool,
    pub allow_rf_disabled_two_tone: bool,
    pub display_frame_limit_hz: u16,
    pub rx_audio_transport_rate_hz: u32,
    pub rx_audio_transport_channels: u32,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            radio_command_addr: SocketAddr::new(
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                COMMAND_DISCOVERY_PORT,
            ),
            client_bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12000),
            tci_bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 50001),
            port_map: P2PortMap::default(),
            enable_discovery: true,
            discovery_timeout: Duration::from_millis(500),
            receive_timeout: Duration::from_millis(100),
            high_priority_period: Duration::from_millis(200),
            rx_ddc_index: 2,
            ddc0_frequency_hz: 14_200_000,
            ddc0_adc: 0,
            ddc0_sample_rate_khz: 192,
            max_client_ddc0_sample_rate_khz: u16::MAX,
            ddc0_sample_size_bits: 24,
            rx_fft_size: 2048,
            rx_low_latency: true,
            tx_fft_size: 4096,
            tx_low_latency: true,
            remote_tx_100w_drive_byte: 68,
            tx_power_meter_scale: 1.0,
            remote_tx_rf_enabled: false,
            allow_rf_disabled_two_tone: false,
            display_frame_limit_hz: 30,
            rx_audio_transport_rate_hz: 48_000,
            rx_audio_transport_channels: 2,
        }
    }
}

impl BridgeConfig {
    pub fn from_env() -> Self {
        let defaults = Self::default();

        let radio_host =
            env::var("SATURN_BRIDGE_RADIO_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let radio_port = parse_env_u16(
            "SATURN_BRIDGE_RADIO_PORT",
            defaults.radio_command_addr.port(),
        );
        let client_host =
            env::var("SATURN_BRIDGE_CLIENT_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let client_port = parse_env_u16(
            "SATURN_BRIDGE_CLIENT_PORT",
            defaults.client_bind_addr.port(),
        );
        let tci_host =
            env::var("SATURN_BRIDGE_TCI_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let tci_port = parse_env_u16("SATURN_BRIDGE_TCI_PORT", defaults.tci_bind_addr.port());

        let max_client_ddc0_sample_rate_khz = parse_env_u16(
            "SATURN_BRIDGE_MAX_CLIENT_DDC0_SAMPLE_RATE_KHZ",
            defaults.max_client_ddc0_sample_rate_khz,
        )
        .max(48);
        let ddc0_sample_rate_khz = parse_env_u16(
            "SATURN_BRIDGE_DDC0_SAMPLE_RATE_KHZ",
            defaults.ddc0_sample_rate_khz,
        )
        .min(max_client_ddc0_sample_rate_khz);

        Self {
            radio_command_addr: parse_socket_addr(
                &radio_host,
                radio_port,
                defaults.radio_command_addr,
            ),
            client_bind_addr: parse_socket_addr(
                &client_host,
                client_port,
                defaults.client_bind_addr,
            ),
            tci_bind_addr: parse_socket_addr(&tci_host, tci_port, defaults.tci_bind_addr),
            port_map: defaults.port_map,
            enable_discovery: parse_env_bool(
                "SATURN_BRIDGE_ENABLE_DISCOVERY",
                defaults.enable_discovery,
            ),
            discovery_timeout: Duration::from_millis(parse_env_u64(
                "SATURN_BRIDGE_DISCOVERY_TIMEOUT_MS",
                defaults.discovery_timeout.as_millis() as u64,
            )),
            receive_timeout: Duration::from_millis(parse_env_u64(
                "SATURN_BRIDGE_RECV_TIMEOUT_MS",
                defaults.receive_timeout.as_millis() as u64,
            )),
            high_priority_period: Duration::from_millis(parse_env_u64(
                "SATURN_BRIDGE_HP_PERIOD_MS",
                defaults.high_priority_period.as_millis() as u64,
            )),
            rx_ddc_index: parse_env_u8("SATURN_BRIDGE_RX_DDC_INDEX", defaults.rx_ddc_index).min(9),
            ddc0_frequency_hz: parse_env_u32(
                "SATURN_BRIDGE_DDC0_FREQUENCY_HZ",
                defaults.ddc0_frequency_hz,
            ),
            ddc0_adc: parse_env_u8("SATURN_BRIDGE_DDC0_ADC", defaults.ddc0_adc).min(2),
            ddc0_sample_rate_khz,
            max_client_ddc0_sample_rate_khz,
            ddc0_sample_size_bits: parse_env_u8(
                "SATURN_BRIDGE_DDC0_SAMPLE_SIZE_BITS",
                defaults.ddc0_sample_size_bits,
            ),
            rx_fft_size: clamp_fft_size(parse_env_u32(
                "SATURN_BRIDGE_RX_FFT_SIZE",
                defaults.rx_fft_size,
            )),
            rx_low_latency: parse_env_bool("SATURN_BRIDGE_RX_LOW_LATENCY", defaults.rx_low_latency),
            tx_fft_size: clamp_fft_size(parse_env_u32(
                "SATURN_BRIDGE_TX_FFT_SIZE",
                defaults.tx_fft_size,
            )),
            tx_low_latency: parse_env_bool("SATURN_BRIDGE_TX_LOW_LATENCY", defaults.tx_low_latency),
            remote_tx_100w_drive_byte: parse_env_u8(
                "SATURN_REMOTE_TX_100W_DRIVE_BYTE",
                defaults.remote_tx_100w_drive_byte,
            )
            .clamp(1, u8::MAX),
            tx_power_meter_scale: parse_env_f32(
                "SATURN_REMOTE_TX_POWER_METER_SCALE",
                defaults.tx_power_meter_scale,
            )
            .clamp(0.1, 2.0),
            remote_tx_rf_enabled: parse_env_bool(
                "SATURN_REMOTE_TX_RF_ENABLED",
                defaults.remote_tx_rf_enabled,
            ),
            allow_rf_disabled_two_tone: parse_env_bool(
                "SATURN_REMOTE_TWOTONE_RF_DISABLED_OK",
                defaults.allow_rf_disabled_two_tone,
            ),
            display_frame_limit_hz: parse_env_u16(
                "SATURN_BRIDGE_DISPLAY_FRAME_LIMIT_HZ",
                defaults.display_frame_limit_hz,
            )
            .min(120),
            rx_audio_transport_rate_hz: parse_env_u32(
                "SATURN_BRIDGE_RX_AUDIO_TRANSPORT_RATE_HZ",
                defaults.rx_audio_transport_rate_hz,
            )
            .clamp(8_000, 48_000),
            rx_audio_transport_channels: parse_env_u32(
                "SATURN_BRIDGE_RX_AUDIO_TRANSPORT_CHANNELS",
                defaults.rx_audio_transport_channels,
            )
            .clamp(1, 2),
        }
    }
}

fn parse_socket_addr(host: &str, port: u16, fallback: SocketAddr) -> SocketAddr {
    let text = format!("{host}:{port}");
    text.parse().unwrap_or(fallback)
}

fn parse_env_bool(name: &str, default: bool) -> bool {
    match env::var(name) {
        Ok(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        },
        Err(_) => default,
    }
}

fn parse_env_u8(name: &str, default: u8) -> u8 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(default)
}

fn parse_env_u16(name: &str, default: u16) -> u16 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(default)
}

fn parse_env_u32(name: &str, default: u32) -> u32 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(default)
}

fn parse_env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn parse_env_f32(name: &str, default: f32) -> f32 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}

/// Clamp an FFT size to the nearest valid power-of-two in [1024, 262144].
/// Follows piHPSDR/Thetis convention: only power-of-two sizes are valid for WDSP.
fn clamp_fft_size(value: u32) -> u32 {
    const MIN_FFT: u32 = 1024;
    const MAX_FFT: u32 = 262144;
    let clamped = value.clamp(MIN_FFT, MAX_FFT);
    // Round down to nearest power of two
    1 << (31 - clamped.leading_zeros())
}
