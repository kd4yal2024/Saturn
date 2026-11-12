//! Offline-only credential issuance at the existing TCI operator boundary.
//! No socket handler calls this yet. TCI operator authorization is NOT a new
//! cryptographic authentication scheme; SATP media authentication stays separate.
#![allow(dead_code)] // intentionally no live socket-handler caller
use super::*;
use saturn_bridge::puresignal::{tx_lifecycle::FreshTxAction, Fault};

pub(crate) struct OperatorIngress {
    clients: ClientRegistry,
    operator: Arc<AtomicU64>,
    next: AtomicU64,
}
// Not Clone: the action capability is consumed exactly once by the owner.
pub(crate) struct Credential {
    live: Arc<std::sync::atomic::AtomicBool>,
    operator: Arc<AtomicU64>,
    client: u64,
    action: FreshTxAction,
    revision: u64,
}
/// Settings permission is sealed at ingress. It is not a TX action and cannot
/// open an output generation. Revocation survives registry removal/promotion.
pub(crate) struct SettingsPermission {
    live: Arc<std::sync::atomic::AtomicBool>,
    operator: Arc<AtomicU64>,
    client: u64,
}
impl SettingsPermission {
    pub(crate) fn valid(&self) -> bool {
        self.client != 0
            && self.operator.load(Ordering::SeqCst) == self.client
            && self.live.load(Ordering::SeqCst)
    }
}
impl Credential {
    pub(crate) fn action(&self) -> FreshTxAction {
        self.action
    }
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
    pub(crate) fn valid(&self) -> bool {
        self.client != 0
            && self.operator.load(Ordering::SeqCst) == self.client
            && self.live.load(Ordering::SeqCst)
    }
}
impl TciFrontend {
    pub(crate) fn offline_ps_ingress(&self) -> OperatorIngress {
        OperatorIngress::from_registry(self.clients.clone(), self.operator_client_id.clone())
    }
}
impl OperatorIngress {
    /// Latest state is replaceable telemetry, never a transaction receipt.
    pub(crate) fn status_reply(&self, text: String) {
        let client = self.operator.load(Ordering::SeqCst);
        if let Some(c) = self.clients.lock_unpoisoned().get(&client) {
            c.outbound.enqueue(OutboundMessage::Text(text));
        }
    }
    pub(crate) fn media_reply(&self, text: String, cancel: impl FnOnce()) {
        let client = self.operator.load(Ordering::SeqCst);
        if client != 0 {
            self.settings_reply(client, text, cancel);
        }
    }
    pub(crate) fn settings_permission(&self, client: u64) -> Result<SettingsPermission, Fault> {
        let live = self
            .clients
            .lock_unpoisoned()
            .get(&client)
            .map(|c| c.state.authorization_live.clone())
            .ok_or(Fault::TxAuthority)?;
        let permission = SettingsPermission {
            live,
            operator: self.operator.clone(),
            client,
        };
        if !permission.valid() {
            return Err(Fault::TxAuthority);
        }
        Ok(permission)
    }
    pub(crate) fn settings_reply(&self, client: u64, text: String, cancel: impl FnOnce()) -> bool {
        let clients = self.clients.lock_unpoisoned();
        let Some(c) = clients.get(&client) else {
            cancel();
            return false;
        };
        if c.outbound.enqueue_transaction_reply(text) {
            return true;
        }
        // Fence closes BEFORE more queue work or session revocation. This
        // callback is atomic-only and cannot wait for native DSP/DMA.
        cancel();
        c.state.authorization_live.store(false, Ordering::SeqCst);
        c.outbound.enqueue(OutboundMessage::Close);
        false
    }
    pub(crate) fn from_registry(clients: ClientRegistry, operator: Arc<AtomicU64>) -> Self {
        Self {
            clients,
            operator,
            next: AtomicU64::new(0),
        }
    }
    pub(crate) fn allowed(&self, client: u64) -> bool {
        client != 0
            && self.operator.load(Ordering::SeqCst) == client
            && self
                .clients
                .try_lock()
                .is_ok_and(|c| c.contains_key(&client))
    }
    pub(crate) fn receive(
        &self,
        client: u64,
        command: &str,
        release_ticket: u64,
        fence: saturn_bridge::puresignal::Fence,
        revision: u64,
    ) -> Result<Credential, Fault> {
        let live = self
            .clients
            .lock_unpoisoned()
            .get(&client)
            .map(|c| c.state.authorization_live.clone())
            .ok_or(Fault::TxAuthority)?;
        let allowed = client != 0
            && self.operator.load(Ordering::SeqCst) == client
            && live.load(Ordering::SeqCst);
        let (send, recv) = std::sync::mpsc::channel();
        // Same role-aware parser as the existing socket ingress. An action is
        // stamped HERE, not when an old queued SetTxEnabled(true) is delivered.
        parse_tci_command_with_roles(
            command,
            &send,
            &self.clients,
            client,
            allowed,
            Some(&self.operator),
        );
        if !allowed || !matches!(recv.try_recv(), Ok(TciCommand::SetTxEnabled(true))) {
            return Err(Fault::TxAuthority);
        }
        let previous = self
            .next
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_add(1))
            .map_err(|_| Fault::TxAuthority)?;
        Ok(Credential {
            live,
            operator: self.operator.clone(),
            client,
            action: FreshTxAction {
                sequence: previous + 1,
                release_ticket,
                expected_fence: fence,
            },
            revision,
        })
    }
    #[cfg(test)]
    pub(crate) fn fixture() -> Self {
        let mut clients = BTreeMap::new();
        for id in [7, 8] {
            clients.insert(
                id,
                ClientConnection {
                    outbound: ClientOutbound::new(),
                    state: ClientState::default(),
                },
            );
        }
        Self {
            clients: Arc::new(Mutex::new(clients)),
            operator: Arc::new(AtomicU64::new(7)),
            next: AtomicU64::new(0),
        }
    }
    #[cfg(test)]
    pub(crate) fn fixture_registry(&self) -> (ClientRegistry, Arc<AtomicU64>) {
        (self.clients.clone(), self.operator.clone())
    }
    #[cfg(test)]
    pub(crate) fn disconnect(&self) {
        unregister_client(&self.clients, &self.operator, 7);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_flood_preserves_authorization_but_receipt_overflow_still_revokes() {
        let ingress = OperatorIngress::fixture();
        let permission = ingress.settings_permission(7).unwrap();
        let cancelled = AtomicBool::new(false);
        for n in 0..1000 {
            ingress.status_reply(format!(
                "saturn_tx_owner:shared,{n},Active,1,2,tci,false,none;"
            ));
            ingress.status_reply(format!(
                "saturn_ps_auto_att:0,1,2,7200000,true,stable,12,12,{n},3,true;"
            ));
        }
        assert!(permission.valid());
        for _ in 0..64 {
            assert!(
                ingress.settings_reply(7, "tx_config:0,1,applied,2;".into(), || cancelled
                    .store(true, Ordering::SeqCst))
            );
        }
        assert!(!cancelled.load(Ordering::SeqCst));
        assert!(
            !ingress.settings_reply(7, "tx_config:0,2,applied,3;".into(), || cancelled
                .store(true, Ordering::SeqCst))
        );
        assert!(cancelled.load(Ordering::SeqCst));
        assert!(!permission.valid());
    }
    #[test]
    fn credential_liveness_is_not_registry_mutex_availability() {
        let ingress = OperatorIngress::fixture();
        let fence = saturn_bridge::puresignal::Fence {
            boot: 1,
            generation: 1,
            config: 1,
            tx_epoch: 1,
        };
        let credential = ingress.receive(7, "trx:0,true", 0, fence, 0).unwrap();
        let held = ingress.clients.lock_unpoisoned();
        assert!(
            credential.valid(),
            "ordinary parser/codec lock is not disconnect evidence"
        );
        drop(held);
        ingress.disconnect();
        assert!(!credential.valid());
    }
    #[test]
    fn removing_session_revokes_even_before_operator_promotion() {
        let ingress = OperatorIngress::fixture();
        let fence = saturn_bridge::puresignal::Fence {
            boot: 1,
            generation: 1,
            config: 1,
            tx_epoch: 1,
        };
        let credential = ingress.receive(7, "trx:0,true", 0, fence, 0).unwrap();
        credential.live.store(false, Ordering::SeqCst);
        assert!(!credential.valid());
    }
}
