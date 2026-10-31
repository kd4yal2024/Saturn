//! Optional Legacy observation route. Not an RF/key permission. The existing
//! RX owner changes/restores layout; the existing TX owner is untouched.
use super::*;
use crate::{
    tx_owner_stop::OutputStamp,
    tx_thread::feedback_owner::{Link, Scope},
    xdma::XdmaError,
    xdma_rx::OperationalRxSession,
};
pub(crate) struct Route {
    link: Link,
    input: Input,
    generation: u64,
    active: Option<Settings>,
    inhibited_until_unkey: bool,
}
impl Route {
    pub(crate) fn attach(rx: &mut OperationalRxSession, input: Input) -> Result<Self, XdmaError> {
        let link = Link::new();
        let origin = Instant::now();
        rx.attach_observer_feedback(link.clone(), input.clone(), origin)?;
        Ok(Self {
            link,
            input,
            generation: 0,
            active: None,
            inhibited_until_unkey: false,
        })
    }
    pub(crate) fn step(
        &mut self,
        keyed: bool,
        two_tone: bool,
        settings: Settings,
    ) -> Result<(), XdmaError> {
        if !keyed {
            self.inhibited_until_unkey = false;
            if self.active.take().is_some() {
                self.input.invalidate();
                let restore = self.link.request(None).map_err(fault)?;
                // Actual TX owner snapshot says unkeyed; monitor cannot create
                // this receipt or send a PTT command itself.
                self.link.permit_restoration(restore);
            }
            return Ok(());
        }
        if let Some(active) = self.active {
            if active != settings || !two_tone || self.input.faulted() {
                self.input.invalidate();
                self.inhibited_until_unkey = true;
            }
            return Ok(()); // Keep layout until actual unkey, including faults.
        }
        if self.inhibited_until_unkey || !two_tone || !self.link.restored().map_err(fault)? {
            return Ok(());
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| fault(saturn_bridge::puresignal::Fault::GenerationMismatch))?;
        self.input.prepare(settings);
        let fence = Fence {
            boot: clock().0,
            generation: self.generation,
            config: self.generation as u128,
            tx_epoch: self.generation,
        };
        // Scope's OutputStamp is ONLY a decoder/layout tag here. There is no
        // output fence, permit, SharedIngress or lifecycle in observer streams.
        self.link
            .request(Some(Scope {
                stimulus: crate::tx_thread::feedback_owner::Stimulus::Unspecified,
                fence,
                route: self.generation as u128,
                frequency_hz: settings.center_hz,
                attenuation_db: settings.attenuation_db,
                output: OutputStamp {
                    generation: self.generation,
                    revision: 0,
                },
            }))
            .map_err(fault)?;
        self.active = Some(settings);
        Ok(())
    }
    pub(crate) fn finish(&mut self, rx: &mut OperationalRxSession) -> Result<(), XdmaError> {
        self.step(false, false, Settings::default())?;
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.link.restored().map_err(fault)? {
            rx.service_managed_feedback()?;
            if Instant::now() >= deadline {
                return Err(XdmaError::Incompatible(
                    "RF monitor restoration deadline; not completed".into(),
                ));
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }
}
fn fault(f: saturn_bridge::puresignal::Fault) -> XdmaError {
    XdmaError::Incompatible(format!("RF monitor acquisition: {f:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        xdma_rx::managed_feedback::tests as acquisition,
        xdma_tx_radio::offline_fixture::DirectFixture,
    };
    #[test]
    fn observer_layout_is_opt_in_keeps_one_reader_restores_edits_and_never_controls_tx() {
        use crate::xdma::ALEX_RX_FILTER_REGISTER;
        // Production RX register layout, ordinary-file fixture only.
        const DDC_RATE_REGISTER: u64 = 0x100c;
        const DDC_INPUT_SELECT_REGISTER: u64 = 0x1010;
        const DDC_STREAM_ENABLE_BIT: u32 = 1 << 30;
        const ADC_ATTENUATION_REGISTER: u64 = 0x2018;
        let files = DirectFixture::new();
        let path = files.register_path();
        files.write(DDC_RATE_REGISTER, 0x10_0000);
        files.write(DDC_INPUT_SELECT_REGISTER, DDC_STREAM_ENABLE_BIT);
        files.write(ALEX_RX_FILTER_REGISTER, 0x1234_4218);
        files.write(ADC_ATTENUATION_REGISTER, 0xabcde);
        let before_tx = files.radio.snapshot();
        let input = Input::new();
        let mut rx = acquisition::session(&path);
        let mut route = Route::attach(&mut rx, input.clone()).unwrap();
        let settings = Settings {
            center_hz: 7_200_000,
            commanded_watts: 5,
            ..Settings::default()
        };
        route.step(false, true, settings).unwrap();
        assert!(route.active.is_none());
        route.step(true, false, settings).unwrap();
        assert!(route.active.is_none());
        route.step(true, true, settings).unwrap();
        acquisition::service(&mut rx).unwrap();
        assert_eq!(files.read(DDC_RATE_REGISTER), 0x10001f);
        assert_eq!(files.read(ALEX_RX_FILTER_REGISTER), 0x1234_1800);
        let token = route.link.desired().scope.unwrap();
        let frames = crate::tx_thread::feedback_stream::tests::direct_frames(256);
        let mut seq = 0;
        acquisition::consume(&mut rx, &frames, &mut seq, Some(token), Instant::now());
        assert_eq!(input.pending.lock().unwrap().active.pairs, 1024);
        rx.tune(7_210_000).unwrap();
        rx.set_rx_attenuation(9).unwrap();
        route
            .step(
                true,
                true,
                Settings {
                    center_hz: 7_210_000,
                    ..settings
                },
            )
            .unwrap();
        assert!(input.suspended.load(Ordering::Acquire));
        assert_eq!(files.read(DDC_RATE_REGISTER), 0x10001f); // wait for ACTUAL unkey
        route.step(false, false, settings).unwrap();
        acquisition::service(&mut rx).unwrap();
        assert!(route.link.restored().unwrap());
        assert_eq!(files.read(DDC_RATE_REGISTER), 0x10_0000);
        assert_eq!(files.read(ADC_ATTENUATION_REGISTER) & 31, 9);
        assert_ne!(files.read(ALEX_RX_FILTER_REGISTER) & 0xffff, 0x1800);
        route.step(true, true, settings).unwrap();
        acquisition::service(&mut rx).unwrap();
        assert_eq!(route.generation, 2);
        assert!(!input.suspended.load(Ordering::Acquire));
        let after_tx = files.radio.snapshot();
        assert_eq!(before_tx.dma_writes, after_tx.dma_writes);
        assert_eq!(before_tx.keyed, after_tx.keyed);
        route.step(false, false, settings).unwrap();
        acquisition::service(&mut rx).unwrap();
        route.finish(&mut rx).unwrap();
    }
}
