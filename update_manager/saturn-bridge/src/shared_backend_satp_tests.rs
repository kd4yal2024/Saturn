//! Actual run_inner, actual control socket and authenticated UDP. No TX request.
use super::*;
use crate::tx_audio::TxAudioSource;
use hmac::{Hmac, Mac};
use sha2::Sha256;

fn wait(mut ready: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(4);
    while !ready() {
        assert!(Instant::now() < end, "SATP control/receiver deadline");
        thread::sleep(Duration::from_millis(1));
    }
}

pub(super) fn client(
    address: std::net::SocketAddr,
    satp_address: std::net::SocketAddr,
    model: &Arc<Mutex<RadioModel>>,
    ingress: &SharedIngress,
) {
    let mut c = Client::open(address);
    c.text("saturn_satp_control:pair;");
    wait(|| {
        c.drain();
        !c.satp_keys.is_empty()
    });
    assert!(c.satp_errors.is_empty());
    let encoded = &c.satp_keys[0];
    assert_eq!(encoded.len(), 64);
    let key: [u8; 32] =
        std::array::from_fn(|i| u8::from_str_radix(&encoded[i * 2..i * 2 + 2], 16).unwrap());

    // Renewal must extend the actual lease, without requesting TX or settings.
    thread::sleep(Duration::from_millis(250));
    let renewed_until = Instant::now() + Duration::from_millis(4900);
    assert!(!model.lock_unpoisoned().satp.live(renewed_until));
    c.text("saturn_satp_control:renew;");
    wait(|| {
        c.drain();
        model.lock_unpoisoned().satp.live(renewed_until)
    });
    assert!(c.transactions.is_empty());

    // A real second socket is a viewer. A subsequent pong proves that its
    // preceding pair/renew requests were processed without issuing a key.
    let stream = std::net::TcpStream::connect(address).unwrap();
    let mut viewer = tungstenite::client(format!("ws://{address}/tci"), stream)
        .unwrap()
        .0;
    viewer.get_mut().set_nonblocking(true).unwrap();
    let generation = model.lock_unpoisoned().satp.generation;
    viewer
        .send(tungstenite::Message::Text(
            "saturn_satp_control:pair;saturn_satp_control:renew;saturn_ping:satp-viewer-probe,1;"
                .into(),
        ))
        .unwrap();
    let mut viewer_role = false;
    wait(|| match viewer.read() {
        Ok(tungstenite::Message::Text(text)) => {
            assert!(
                !text.contains("saturn_satp_key:"),
                "pairing key leaked to viewer"
            );
            viewer_role |= text.contains("remote_client_role:0,viewer,");
            text.contains("saturn_pong:satp-viewer-probe,1;")
        }
        Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => false,
        Ok(_) => false,
        Err(_) => panic!("viewer fixture socket failed"),
    });
    assert!(viewer_role);
    assert_eq!(model.lock_unpoisoned().satp.generation, generation);
    let _ = viewer.get_mut().shutdown(std::net::Shutdown::Both);

    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    let audio_stop = Arc::new(AtomicBool::new(false));
    let stop = audio_stop.clone();
    struct Audio {
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
    }
    impl Drop for Audio {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            self.worker.take().unwrap().join().unwrap();
        }
    }
    let audio = Audio {
        stop: audio_stop,
        worker: Some(thread::spawn(move || {
            let mut sequence = 0u32;
            while !stop.load(Ordering::Acquire) {
                let mut bytes = [0u8; 576];
                bytes[..4].copy_from_slice(b"SAT1");
                bytes[4..8].copy_from_slice(&[2, 1, 1, 1]);
                bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
                bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
                bytes[16..20].copy_from_slice(&sequence.to_le_bytes());
                let counter = u64::from(sequence) * 128;
                bytes[20..28].copy_from_slice(&counter.to_le_bytes());
                bytes[28..30].copy_from_slice(&128u16.to_le_bytes());
                for i in 0..128 {
                    let sample = 0.1
                        * (std::f64::consts::TAU * 700. * (counter + i as u64) as f64 / 48000.)
                            .sin() as f32;
                    bytes[32 + i * 4..36 + i * 4].copy_from_slice(&sample.to_le_bytes());
                }
                let mut mac = Hmac::<Sha256>::new_from_slice(&key).unwrap();
                mac.update(&bytes[..544]);
                bytes[544..].copy_from_slice(&mac.finalize().into_bytes());
                udp.send_to(&bytes, satp_address).unwrap();
                sequence += 1;
                thread::sleep(Duration::from_nanos(2_666_667));
            }
        })),
    };
    wait(|| {
        c.drain();
        model.lock_unpoisoned().satp.ready(Instant::now())
    });
    c.text("saturn_satp_control:satp;");
    wait(|| {
        c.drain();
        model.lock_unpoisoned().satp.source == TxAudioSource::Satp
            && ingress.startup_ready()
            && c.transactions.iter().any(|r| r.contains(",applied,"))
    });
    assert!(c.satp_errors.is_empty());
    assert_eq!(ingress.lifecycle_status().unwrap().phase, Phase::ReadyOff);
    assert!(!model.lock_unpoisoned().desired.tx_enabled);

    c.text("saturn_satp_control:tci;");
    wait(|| {
        c.drain();
        model.lock_unpoisoned().satp.source == TxAudioSource::Tci
            && ingress.startup_ready()
            && c.transactions
                .iter()
                .filter(|r| r.contains(",applied,"))
                .count()
                >= 2
    });
    drop(audio);
    drop(c);
    wait(|| !model.lock_unpoisoned().satp.live(Instant::now()));
    assert!(!model.lock_unpoisoned().desired.tx_enabled);
    eprintln!("actual run_inner SATP pair/renew/authenticated UDP/source return/disconnect passed; no TX request; key not logged");
}

#[test]
#[ignore = "serial matched WDSP actual runtime, control socket and authenticated UDP on both file/loopback backends; no TX"]
fn actual_run_inner_satp_pair_renew_and_source_both_backends() {
    for direct in [true, false] {
        exercise(direct, Case::SatpPairing);
    }
}
