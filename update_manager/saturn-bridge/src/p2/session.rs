use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::config::BridgeConfig;
use crate::p2::packets::{
    build_ddc_specific_packet, build_discovery_request, build_duc_iq_packet,
    build_duc_specific_packet, build_general_packet, build_high_priority_to_sdr,
    parse_ddc_iq_frame, parse_discovery_reply, parse_high_priority_from_sdr, DdcIqFrame, DdcSetup,
    DiscoveryReply, HighPriorityFromSdr, HighPriorityToSdr, DUC_IQ_SAMPLES,
};
use crate::p2::ports::{P2PortMap, COMMAND_DISCOVERY_PORT};
use crate::radio_model::RadioModel;

const ALEX_TX_RELAY_BIT: u16 = 0x0800;

#[derive(Debug)]
pub enum P2Event {
    HighPriorityFromSdr(HighPriorityFromSdr),
    DdcIq(DdcIqFrame),
}

pub struct P2Session {
    config: BridgeConfig,
    socket: UdpSocket,
    duc_iq_sequence: AtomicU32,
    duc_specific_sequence: AtomicU32,
}

impl P2Session {
    pub fn bind(config: BridgeConfig) -> io::Result<Self> {
        let socket = UdpSocket::bind(config.client_bind_addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            config,
            socket,
            duc_iq_sequence: AtomicU32::new(0),
            duc_specific_sequence: AtomicU32::new(0),
        })
    }

    pub fn client_bind_addr(&self) -> SocketAddr {
        self.config.client_bind_addr
    }

    pub fn configure_rx_ddc(
        &self,
        ddc_index: u8,
        sample_rate_khz: u16,
        sample_size_bits: u8,
        adc: u8,
    ) -> io::Result<()> {
        self.send_packet(
            self.target_addr(self.config.port_map.ddc_specific),
            &build_ddc_specific_packet(DdcSetup {
                enable_mask: 1u16 << ddc_index.min(9),
                ddc_index,
                adc,
                sample_rate_khz,
                sample_size_bits,
            }),
        )?;
        Ok(())
    }

    pub fn bootstrap(&self, radio_model: &Arc<Mutex<RadioModel>>) -> io::Result<()> {
        if self.config.enable_discovery {
            let _ = self.try_discover(radio_model)?;
        }

        self.send_packet(
            self.config.radio_command_addr,
            &build_general_packet(&self.config.port_map),
        )?;

        let ddc_setup = {
            let model = radio_model.lock().unwrap();
            DdcSetup {
                enable_mask: 1u16 << model.desired.rx_ddc_index,
                ddc_index: model.desired.rx_ddc_index,
                adc: model.desired.ddc0_adc,
                sample_rate_khz: model.desired.ddc0_sample_rate_khz,
                sample_size_bits: model.desired.ddc0_sample_size_bits,
            }
        };
        self.send_packet(
            self.target_addr(self.config.port_map.ddc_specific),
            &build_ddc_specific_packet(ddc_setup),
        )?;
        self.send_duc_specific()?;
        Ok(())
    }

    pub fn spawn_high_priority_loop(
        &self,
        radio_model: Arc<Mutex<RadioModel>>,
        stop_flag: Arc<AtomicBool>,
    ) -> io::Result<JoinHandle<io::Result<()>>> {
        let socket = self.socket.try_clone()?;
        let target = self.target_addr(self.config.port_map.high_priority_to_sdr);
        let rx_period = self.config.high_priority_period;
        let tx_period = rx_period.min(Duration::from_millis(10));

        Ok(thread::spawn(move || {
            let mut prev_tx = false;
            while !stop_flag.load(Ordering::Relaxed) {
                let state = {
                    let model = radio_model.lock().unwrap();
                    build_high_priority_state(&model)
                };

                if state.tx != prev_tx {
                    println!(
                        "saturn-bridge: HP loop MOX -> {} (byte4=0x{:02x})",
                        if state.tx { "TX" } else { "RX" },
                        if state.run { 0x01u8 } else { 0u8 } | if state.tx { 0x02 } else { 0 }
                    );
                    prev_tx = state.tx;
                }

                if state.run {
                    let packet = build_high_priority_to_sdr(&state);
                    socket.send_to(&packet, target)?;
                }
                thread::sleep(if state.tx { tx_period } else { rx_period });
            }
            Ok(())
        }))
    }

    pub fn send_high_priority(&self, model: &RadioModel) -> io::Result<()> {
        let packet = build_high_priority_to_sdr(&build_high_priority_state(model));
        self.send_packet(
            self.target_addr(self.config.port_map.high_priority_to_sdr),
            &packet,
        )?;
        Ok(())
    }

    pub fn recv_event(&self) -> io::Result<Option<P2Event>> {
        let mut buffer = [0u8; 2048];

        match self.socket.recv_from(&mut buffer) {
            Ok((size, source)) => Ok(self.decode_event(&buffer[..size], source.port())),
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Send a single run=false high-priority packet to cleanly release controller ownership.
    /// Call this when the last TCI client disconnects so the SDR goes idle immediately
    /// rather than waiting for its watchdog to expire.
    pub fn send_stop(&self) -> io::Result<()> {
        let packet = build_high_priority_to_sdr(&HighPriorityToSdr {
            run: false,
            tx: false,
            ddc_phase_words: [0u32; 10],
            duc_phase_word: 0,
            tx_drive: 0,
            cat_port: 0,
            alex_tx_word: 0,
            alex_rx_word: 0,
            alex_rx2_filter_word: 0,
            alex_rx1_filter_word: 0,
            rx2_attenuation_db: 0,
            rx1_attenuation_db: 0,
        });
        self.send_packet(
            self.target_addr(self.config.port_map.high_priority_to_sdr),
            &packet,
        )?;
        Ok(())
    }

    /// Send DUC IQ samples to the radio in 240-sample (1444-byte) packets.
    /// `iq_samples` is interleaved [I, Q, I, Q, …] f32 values; only complete
    /// 240-pair chunks are transmitted — any trailing partial chunk is dropped.
    pub fn send_duc_iq(&self, iq_samples: &[f32]) -> io::Result<()> {
        let target = self.target_addr(self.config.port_map.duc_iq);
        for chunk in iq_samples.chunks(DUC_IQ_SAMPLES * 2) {
            if chunk.len() < DUC_IQ_SAMPLES * 2 {
                break;
            }
            let seq = self.duc_iq_sequence.fetch_add(1, Ordering::Relaxed);
            let packet = build_duc_iq_packet(seq, chunk);
            self.send_packet(target, &packet)?;
        }
        Ok(())
    }

    pub fn send_duc_specific(&self) -> io::Result<()> {
        let target = self.target_addr(self.config.port_map.duc_specific);
        let seq = self.duc_specific_sequence.fetch_add(1, Ordering::Relaxed);
        let packet = build_duc_specific_packet(seq);
        self.send_packet(target, &packet)?;
        Ok(())
    }

    pub fn target_addr(&self, port: u16) -> SocketAddr {
        SocketAddr::new(self.config.radio_command_addr.ip(), port)
    }

    fn send_packet(&self, target: SocketAddr, packet: &[u8]) -> io::Result<usize> {
        self.socket.send_to(packet, target)
    }

    fn try_discover(
        &self,
        radio_model: &Arc<Mutex<RadioModel>>,
    ) -> io::Result<Option<DiscoveryReply>> {
        self.send_packet(self.config.radio_command_addr, &build_discovery_request())?;
        let deadline = Instant::now() + self.config.discovery_timeout;
        let mut buffer = [0u8; 2048];

        while Instant::now() < deadline {
            match self.socket.recv_from(&mut buffer) {
                Ok((size, source))
                    if source.port() == COMMAND_DISCOVERY_PORT
                        && size == crate::p2::packets::DISCOVERY_REPLY_SIZE =>
                {
                    if let Some(reply) = parse_discovery_reply(&buffer[..size]) {
                        radio_model.lock().unwrap().apply_discovery(reply.clone());
                        return Ok(Some(reply));
                    }
                }
                Ok(_) => {}
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::TimedOut =>
                {
                    thread::sleep(self.config.receive_timeout.min(Duration::from_millis(5)));
                    continue;
                }
                Err(error) => return Err(error),
            }
        }

        Ok(None)
    }

    fn decode_event(&self, packet: &[u8], source_port: u16) -> Option<P2Event> {
        if source_port == self.config.port_map.high_priority_from_sdr {
            return parse_high_priority_from_sdr(packet).map(P2Event::HighPriorityFromSdr);
        }

        if let Some(ddc_index) = self.config.port_map.ddc_index_from_source_port(source_port) {
            return parse_ddc_iq_frame(packet, ddc_index).map(P2Event::DdcIq);
        }

        None
    }
}

fn build_ddc_phase_array(model: &RadioModel) -> [u32; 10] {
    let mut phase_words = [0u32; 10];
    phase_words[usize::from(model.desired.rx_ddc_index)] =
        frequency_to_phase_word(model.desired.iq_center_hz);
    phase_words
}

fn build_high_priority_state(model: &RadioModel) -> HighPriorityToSdr {
    HighPriorityToSdr {
        run: model.desired.running,
        tx: model.desired.tx_enabled,
        ddc_phase_words: build_ddc_phase_array(model),
        duc_phase_word: frequency_to_phase_word(model.desired.tx_frequency_hz),
        tx_drive: (model.desired.tx_drive as u16 * 255 / 100) as u8,
        cat_port: 0,
        alex_tx_word: build_alex_tx_word(model.desired.tx_frequency_hz, 1),
        alex_rx_word: build_alex_legacy_tx_word(
            model.desired.tx_frequency_hz,
            model.desired.rx_antenna,
            model.desired.tx_enabled,
        ),
        alex_rx2_filter_word: build_alex_rx_filter_word(model.desired.iq_center_hz),
        alex_rx1_filter_word: build_alex_rx_filter_word(model.desired.iq_center_hz),
        rx2_attenuation_db: 0,
        rx1_attenuation_db: 0,
    }
}

fn frequency_to_phase_word(frequency_hz: u32) -> u32 {
    let clamped = frequency_hz.min(122_880_000);
    ((clamped as f64) * (4_294_967_296.0 / 122_880_000.0)) as u32
}

fn build_alex_tx_word(frequency_hz: u32, antenna: u8) -> u16 {
    alex_tx_filter_bits(frequency_hz) | alex_antenna_bits(antenna) | ALEX_TX_RELAY_BIT
}

fn build_alex_legacy_tx_word(frequency_hz: u32, antenna: u8, tx_active: bool) -> u16 {
    let mut word = alex_tx_filter_bits(frequency_hz) | alex_antenna_bits(antenna);
    if tx_active {
        word |= ALEX_TX_RELAY_BIT;
    }
    word
}

fn build_alex_rx_filter_word(frequency_hz: u32) -> u16 {
    alex_rx_filter_bits(frequency_hz)
}

fn alex_antenna_bits(antenna: u8) -> u16 {
    match antenna {
        2 => 0x0200,
        3 => 0x0400,
        _ => 0x0100,
    }
}

fn alex_rx_filter_bits(frequency_hz: u32) -> u16 {
    match frequency_hz {
        0..=1_499_999 => 0x1000,
        1_500_000..=2_099_999 => 0x0040,
        2_100_000..=5_499_999 => 0x0020,
        5_500_000..=10_999_999 => 0x0010,
        11_000_000..=21_999_999 => 0x0002,
        22_000_000..=34_999_999 => 0x0004,
        _ => 0x0008,
    }
}

fn alex_tx_filter_bits(frequency_hz: u32) -> u16 {
    match frequency_hz {
        35_600_001..=u32::MAX => 0x2000,
        24_000_001..=35_600_000 => 0x4000,
        16_500_001..=24_000_000 => 0x8000,
        8_000_001..=16_500_000 => 0x0010,
        5_000_001..=8_000_000 => 0x0020,
        2_500_001..=5_000_000 => 0x0040,
        _ => 0x0080,
    }
}

#[allow(dead_code)]
fn _ports(_port_map: &P2PortMap) -> Duration {
    Duration::from_millis(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::radio_model::RadioModel;

    #[test]
    fn active_rx_ddc_uses_selected_slot_and_delta_phase() {
        let mut model = RadioModel::new(2, 14_200_000, 0, 192, 24);
        model.desired.rx_ddc_index = 2;
        model.desired.iq_center_hz = 14_200_000;

        let phase_words = build_ddc_phase_array(&model);

        assert_eq!(phase_words[0], 0);
        assert_eq!(phase_words[1], 0);
        assert_eq!(phase_words[2], frequency_to_phase_word(14_200_000));
    }

    #[test]
    fn saturn_filter_maps_match_pihpsdr_ranges() {
        assert_eq!(alex_rx_filter_bits(14_200_000), 0x0002);
        assert_eq!(alex_rx_filter_bits(10_000_000), 0x0010);
        assert_eq!(alex_tx_filter_bits(14_200_000), 0x0010);
        assert_eq!(alex_tx_filter_bits(3_900_000), 0x0040);
    }

    #[test]
    fn saturn_tx_words_carry_pa_and_tx_relay_bits() {
        assert_eq!(
            build_alex_tx_word(14_200_000, 1) & ALEX_TX_RELAY_BIT,
            ALEX_TX_RELAY_BIT
        );
        assert_eq!(
            build_alex_legacy_tx_word(14_200_000, 1, false) & ALEX_TX_RELAY_BIT,
            0
        );
        assert_eq!(
            build_alex_legacy_tx_word(14_200_000, 1, true) & ALEX_TX_RELAY_BIT,
            ALEX_TX_RELAY_BIT
        );
    }
}
