use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::sync_ext::MutexExt;

use super::*;

#[derive(Clone, Debug)]
pub(crate) struct SplitClientMetadata {
    pub(crate) session_id: String,
    pub(crate) lane: Option<SplitSocketKind>,
    pub(crate) role: Option<TciClientRole>,
    pub(crate) ignore_media_until: Option<Instant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SplitSessionPair {
    pub(crate) session_id: String,
    pub(crate) control_client_id: u64,
    pub(crate) media_client_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TciClientRole {
    Operator,
    Viewer,
}

impl TciClientRole {
    pub(crate) fn as_tci(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::Viewer => "viewer",
        }
    }

    pub(crate) fn from_tci(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "viewer" | "view" => Self::Viewer,
            _ => Self::Operator,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitSocketKind {
    Control,
    Media,
}

impl SplitSocketKind {
    #[cfg(test)]
    pub(crate) fn as_tci(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Media => "media",
        }
    }

    pub(crate) fn from_tci(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "control" => Some(Self::Control),
            "media" => Some(Self::Media),
            _ => None,
        }
    }
}

pub(crate) fn remote_client_role_message(client_id: u64, role: TciClientRole) -> String {
    format!("remote_client_role:0,{},{client_id};", role.as_tci())
}

#[cfg(test)]
pub(crate) const SPLIT_SESSION_PAIRING_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) const SPLIT_RELEASE_IGNORE_WINDOW: Duration = Duration::from_millis(250);

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SplitSessionState {
    WaitingMedia,
    Paired,
    Keyed,
    Terminated,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SplitMediaFrameAction {
    Accept,
    DropNotKeyed,
    DropReleaseWindow,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SplitDisconnectAction {
    pub(crate) force_rx: bool,
    pub(crate) close_peer_socket: bool,
    pub(crate) state: SplitSessionState,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct SplitSession {
    pub(crate) session_id: String,
    pub(crate) state: SplitSessionState,
    pub(crate) control_connected: bool,
    pub(crate) media_connected: bool,
    pub(crate) created_at: Instant,
    pub(crate) ignore_media_until: Option<Instant>,
    pub(crate) release_window_drops: u64,
}

#[cfg(test)]
impl SplitSession {
    pub(crate) fn new_control(session_id: &str, now: Instant) -> Option<Self> {
        let session_id = normalize_split_session_id(session_id)?;
        Some(Self {
            session_id,
            state: SplitSessionState::WaitingMedia,
            control_connected: true,
            media_connected: false,
            created_at: now,
            ignore_media_until: None,
            release_window_drops: 0,
        })
    }

    pub(crate) fn connect_media(&mut self) -> Option<String> {
        if self.state == SplitSessionState::Terminated {
            return None;
        }
        self.media_connected = true;
        if self.control_connected {
            self.state = SplitSessionState::Paired;
            return Some(split_session_paired_message(&self.session_id));
        }
        None
    }

    pub(crate) fn pairing_timed_out(&self, now: Instant) -> bool {
        self.state == SplitSessionState::WaitingMedia
            && now.saturating_duration_since(self.created_at) >= SPLIT_SESSION_PAIRING_TIMEOUT
    }

    pub(crate) fn key(&mut self) -> bool {
        if self.state != SplitSessionState::Paired {
            return false;
        }
        self.state = SplitSessionState::Keyed;
        true
    }

    pub(crate) fn release(&mut self, now: Instant) -> bool {
        if self.state != SplitSessionState::Keyed {
            return false;
        }
        self.state = SplitSessionState::Paired;
        self.ignore_media_until = Some(now + SPLIT_RELEASE_IGNORE_WINDOW);
        true
    }

    pub(crate) fn media_frame_action(&mut self, now: Instant) -> SplitMediaFrameAction {
        if self
            .ignore_media_until
            .map(|until| now < until)
            .unwrap_or(false)
        {
            self.release_window_drops = self.release_window_drops.saturating_add(1);
            return SplitMediaFrameAction::DropReleaseWindow;
        }
        if self.state == SplitSessionState::Keyed && self.media_connected {
            SplitMediaFrameAction::Accept
        } else {
            SplitMediaFrameAction::DropNotKeyed
        }
    }

    pub(crate) fn disconnect_control(&mut self) -> SplitDisconnectAction {
        self.control_connected = false;
        self.media_connected = false;
        self.state = SplitSessionState::Terminated;
        SplitDisconnectAction {
            force_rx: true,
            close_peer_socket: true,
            state: self.state,
        }
    }

    pub(crate) fn disconnect_media(&mut self) -> SplitDisconnectAction {
        self.media_connected = false;
        let was_keyed = self.state == SplitSessionState::Keyed;
        if self.state != SplitSessionState::Terminated {
            self.state = SplitSessionState::WaitingMedia;
        }
        SplitDisconnectAction {
            force_rx: was_keyed,
            close_peer_socket: false,
            state: self.state,
        }
    }
}

pub(crate) fn normalize_split_session_id(value: &str) -> Option<String> {
    let session_id = sanitize_token(value, 64);
    if session_id.is_empty() {
        None
    } else {
        Some(session_id)
    }
}

#[cfg(test)]
pub(crate) fn split_session_paired_message(session_id: &str) -> String {
    format!(
        "session_paired:{};",
        normalize_split_session_id(session_id).unwrap_or_default()
    )
}

pub(crate) fn parse_split_session_open(command: &str) -> Option<(String, TciClientRole)> {
    let command = command.trim().trim_end_matches(';');
    let (name, rest) = command.split_once(':')?;
    if !name.eq_ignore_ascii_case("session_open") {
        return None;
    }
    let mut args = rest.split(',');
    let session_id = normalize_split_session_id(args.next().unwrap_or_default())?;
    let role = args
        .next()
        .map(TciClientRole::from_tci)
        .unwrap_or(TciClientRole::Operator);
    Some((session_id, role))
}

pub(crate) fn parse_split_session_lane(command: &str) -> Option<(String, SplitSocketKind)> {
    let command = command.trim().trim_end_matches(';');
    let (name, rest) = command.split_once(':')?;
    if !name.eq_ignore_ascii_case("session_lane") {
        return None;
    }
    let mut args = rest.split(',');
    let session_id = normalize_split_session_id(args.next().unwrap_or_default())?;
    let lane = SplitSocketKind::from_tci(args.next()?)?;
    Some((session_id, lane))
}

pub(crate) fn reconcile_split_operator_role(
    clients: &ClientRegistry,
    operator_client_id: &Arc<AtomicU64>,
    changed_client_id: u64,
) {
    let current_operator = operator_client_id.load(Ordering::SeqCst);
    let outcome = (|| {
        let clients = clients.lock_unpoisoned();
        let changed_metadata = clients.get(&changed_client_id)?.state.split.as_ref()?;
        let pair = split_session_pair_in_clients(&clients, &changed_metadata.session_id)?;
        let control = clients.get(&pair.control_client_id)?;
        let control_metadata = control.state.split.as_ref()?;
        if control_metadata.role != Some(TciClientRole::Operator)
            || current_operator == pair.control_client_id
        {
            return None;
        }

        let current_operator_missing =
            current_operator == 0 || !clients.contains_key(&current_operator);
        let current_operator_is_paired_media = current_operator == pair.media_client_id;
        if !current_operator_missing && !current_operator_is_paired_media {
            return None;
        }

        Some((
            pair.session_id,
            pair.control_client_id,
            current_operator,
            control.outbound.clone(),
        ))
    })();

    let Some((session_id, control_client_id, previous_operator, control_outbound)) = outcome else {
        return;
    };

    operator_client_id.store(control_client_id, Ordering::SeqCst);
    let _ = control_outbound.enqueue(OutboundMessage::SafetyText(remote_client_role_message(
        control_client_id,
        TciClientRole::Operator,
    )));
    println!(
        "saturn-bridge: Phase 42 session {session_id} moved operator role from client {previous_operator} to control client {control_client_id}"
    );
}

// Phase 42 lane-aware routing helper. Given a control-lane client_id,
// returns the paired media-lane client_id if both halves of the split
// session are connected and registered. Used to propagate RX stream-enable
// state from the control client (which receives iq_start/audio_start text)
// to the media client (which is the destination for binary RX frames).
// Returns None for non-Phase-42 clients, unpaired sessions, or when called
// on a media-lane client.
pub(crate) fn split_paired_media_client_id(
    clients: &BTreeMap<u64, ClientConnection>,
    control_client_id: u64,
) -> Option<u64> {
    let metadata = clients.get(&control_client_id)?.state.split.as_ref()?;
    if metadata.lane != Some(SplitSocketKind::Control) {
        return None;
    }
    let pair = split_session_pair_in_clients(clients, &metadata.session_id)?;
    if pair.control_client_id != control_client_id {
        return None;
    }
    Some(pair.media_client_id)
}

pub(crate) fn set_client_split_session_open(
    clients: &ClientRegistry,
    client_id: u64,
    session_id: &str,
    role: TciClientRole,
) -> bool {
    let Some(session_id) = normalize_split_session_id(session_id) else {
        return false;
    };
    let mut clients = clients.lock_unpoisoned();
    let Some(client) = clients.get_mut(&client_id) else {
        return false;
    };
    let metadata = client
        .state
        .split
        .get_or_insert_with(|| SplitClientMetadata {
            session_id: session_id.clone(),
            lane: None,
            role: None,
            ignore_media_until: None,
        });
    if metadata.session_id != session_id {
        return false;
    }
    metadata.role = Some(role);
    true
}

pub(crate) fn set_client_split_session_lane(
    clients: &ClientRegistry,
    client_id: u64,
    session_id: &str,
    lane: SplitSocketKind,
) -> bool {
    let Some(session_id) = normalize_split_session_id(session_id) else {
        return false;
    };
    let mut clients = clients.lock_unpoisoned();
    let Some(client) = clients.get_mut(&client_id) else {
        return false;
    };
    let metadata = client
        .state
        .split
        .get_or_insert_with(|| SplitClientMetadata {
            session_id: session_id.clone(),
            lane: None,
            role: None,
            ignore_media_until: None,
        });
    if metadata.session_id != session_id {
        return false;
    }
    metadata.lane = Some(lane);
    true
}

#[cfg(test)]
pub(crate) fn split_session_pair_for_client(
    clients: &ClientRegistry,
    client_id: u64,
) -> Option<SplitSessionPair> {
    let clients = clients.lock_unpoisoned();
    let session_id = clients
        .get(&client_id)?
        .state
        .split
        .as_ref()?
        .session_id
        .clone();
    split_session_pair_in_clients(&clients, &session_id)
}

pub(crate) fn split_media_client_can_supply_mic(
    clients: &ClientRegistry,
    operator_client_id: u64,
    media_client_id: u64,
    now: Instant,
) -> bool {
    if operator_client_id == 0 || operator_client_id == media_client_id {
        return false;
    }
    let clients = clients.lock_unpoisoned();
    let Some(pair) = split_session_pair_for_client_in_clients(&clients, media_client_id) else {
        return false;
    };
    if pair.control_client_id != operator_client_id || pair.media_client_id != media_client_id {
        return false;
    }
    !clients
        .get(&media_client_id)
        .and_then(|client| client.state.split.as_ref())
        .and_then(|metadata| metadata.ignore_media_until)
        .map(|until| now < until)
        .unwrap_or(false)
}

pub(crate) fn split_media_client_paired_with_operator_in_clients(
    clients: &BTreeMap<u64, ClientConnection>,
    operator_client_id: u64,
    media_client_id: u64,
) -> bool {
    if operator_client_id == 0 || operator_client_id == media_client_id {
        return false;
    }
    split_session_pair_for_client_in_clients(clients, media_client_id)
        .map(|pair| {
            pair.control_client_id == operator_client_id && pair.media_client_id == media_client_id
        })
        .unwrap_or(false)
}

pub(crate) fn queue_split_media_peer_close_for_control_in_clients(
    clients: &BTreeMap<u64, ClientConnection>,
    control_client_id: u64,
) -> Option<u64> {
    let metadata = clients.get(&control_client_id)?.state.split.as_ref()?;
    if metadata.lane != Some(SplitSocketKind::Control) {
        return None;
    }
    let pair = split_session_pair_in_clients(clients, &metadata.session_id)?;
    if pair.control_client_id != control_client_id {
        return None;
    }
    let media = clients.get(&pair.media_client_id)?;
    let _ = media.outbound.enqueue(OutboundMessage::Close);
    Some(pair.media_client_id)
}

pub(crate) fn client_is_split_media(client: &ClientConnection) -> bool {
    client
        .state
        .split
        .as_ref()
        .and_then(|metadata| metadata.lane)
        == Some(SplitSocketKind::Media)
}

pub(crate) fn split_session_pair_for_client_in_clients(
    clients: &BTreeMap<u64, ClientConnection>,
    client_id: u64,
) -> Option<SplitSessionPair> {
    let session_id = clients
        .get(&client_id)?
        .state
        .split
        .as_ref()?
        .session_id
        .clone();
    split_session_pair_in_clients(clients, &session_id)
}

pub(crate) fn set_split_media_ignore_until(
    clients: &ClientRegistry,
    operator_client_id: u64,
    ignore_until: Option<Instant>,
) -> u64 {
    if operator_client_id == 0 {
        return 0;
    }
    let mut clients = clients.lock_unpoisoned();
    let Some(operator_session_id) = clients
        .get(&operator_client_id)
        .and_then(|client| client.state.split.as_ref())
        .filter(|metadata| metadata.lane == Some(SplitSocketKind::Control))
        .map(|metadata| metadata.session_id.clone())
    else {
        return 0;
    };

    let mut updated = 0;
    for client in clients.values_mut() {
        let Some(metadata) = client.state.split.as_mut() else {
            continue;
        };
        if metadata.session_id == operator_session_id
            && metadata.lane == Some(SplitSocketKind::Media)
        {
            metadata.ignore_media_until = ignore_until;
            updated += 1;
        }
    }
    updated
}

pub(crate) fn split_session_pair_in_clients(
    clients: &BTreeMap<u64, ClientConnection>,
    session_id: &str,
) -> Option<SplitSessionPair> {
    let mut control_client_id = None;
    let mut media_client_id = None;
    for (&client_id, client) in clients {
        let Some(metadata) = client.state.split.as_ref() else {
            continue;
        };
        if metadata.session_id != session_id {
            continue;
        }
        match metadata.lane {
            Some(SplitSocketKind::Control) => control_client_id.get_or_insert(client_id),
            Some(SplitSocketKind::Media) => media_client_id.get_or_insert(client_id),
            None => continue,
        };
    }
    Some(SplitSessionPair {
        session_id: session_id.to_string(),
        control_client_id: control_client_id?,
        media_client_id: media_client_id?,
    })
}

pub(crate) fn split_lane_client_count(
    clients: &BTreeMap<u64, ClientConnection>,
    lane: SplitSocketKind,
) -> u64 {
    clients
        .values()
        .filter(|client| {
            client
                .state
                .split
                .as_ref()
                .and_then(|metadata| metadata.lane)
                == Some(lane)
        })
        .count() as u64
}

pub(crate) fn split_paired_session_count(clients: &BTreeMap<u64, ClientConnection>) -> u64 {
    let mut control_sessions = BTreeSet::new();
    let mut media_sessions = BTreeSet::new();
    for client in clients.values() {
        let Some(metadata) = client.state.split.as_ref() else {
            continue;
        };
        match metadata.lane {
            Some(SplitSocketKind::Control) => {
                control_sessions.insert(metadata.session_id.clone());
            }
            Some(SplitSocketKind::Media) => {
                media_sessions.insert(metadata.session_id.clone());
            }
            None => {}
        }
    }
    control_sessions.intersection(&media_sessions).count() as u64
}
