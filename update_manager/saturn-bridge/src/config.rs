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
    pub ddc0_sample_size_bits: u8,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            radio_command_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), COMMAND_DISCOVERY_PORT),
            client_bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12000),
            tci_bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 50001),
            port_map: P2PortMap::default(),
            enable_discovery: true,
            discovery_timeout: Duration::from_millis(500),
            receive_timeout: Duration::from_millis(100),
            high_priority_period: Duration::from_millis(200),
            rx_ddc_index: 2,
            ddc0_frequency_hz: 14_200_000,
            ddc0_adc: 0,
            ddc0_sample_rate_khz: 192,
            ddc0_sample_size_bits: 24,
        }
    }
}

impl BridgeConfig {
    pub fn from_env() -> Self {
        let defaults = Self::default();

        let radio_host = env::var("SATURN_BRIDGE_RADIO_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let radio_port = parse_env_u16("SATURN_BRIDGE_RADIO_PORT", defaults.radio_command_addr.port());
        let client_host = env::var("SATURN_BRIDGE_CLIENT_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let client_port = parse_env_u16("SATURN_BRIDGE_CLIENT_PORT", defaults.client_bind_addr.port());
        let tci_host = env::var("SATURN_BRIDGE_TCI_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let tci_port = parse_env_u16("SATURN_BRIDGE_TCI_PORT", defaults.tci_bind_addr.port());

        Self {
            radio_command_addr: parse_socket_addr(&radio_host, radio_port, defaults.radio_command_addr),
            client_bind_addr: parse_socket_addr(&client_host, client_port, defaults.client_bind_addr),
            tci_bind_addr: parse_socket_addr(&tci_host, tci_port, defaults.tci_bind_addr),
            port_map: defaults.port_map,
            enable_discovery: parse_env_bool("SATURN_BRIDGE_ENABLE_DISCOVERY", defaults.enable_discovery),
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
            ddc0_frequency_hz: parse_env_u32("SATURN_BRIDGE_DDC0_FREQUENCY_HZ", defaults.ddc0_frequency_hz),
            ddc0_adc: parse_env_u8("SATURN_BRIDGE_DDC0_ADC", defaults.ddc0_adc).min(2),
            ddc0_sample_rate_khz: parse_env_u16(
                "SATURN_BRIDGE_DDC0_SAMPLE_RATE_KHZ",
                defaults.ddc0_sample_rate_khz,
            ),
            ddc0_sample_size_bits: parse_env_u8(
                "SATURN_BRIDGE_DDC0_SAMPLE_SIZE_BITS",
                defaults.ddc0_sample_size_bits,
            ),
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
