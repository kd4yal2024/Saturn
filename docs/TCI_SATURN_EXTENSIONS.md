# Saturn TCI Extensions

Status: Phase 0E protocol freeze

Saturn uses standard TCI 2.0 commands for radio control and state. Commands in
this document are Saturn extensions used only to discover the separate SATP
native-audio plane. They do not redefine standard TCI audio.

## Server announcement

Saturn Bridge includes these semicolon-terminated commands in the initial TCI
state burst, before `ready;`:

```text
saturn_satp_supported:true;
saturn_satp_enabled:true;
saturn_satp_version:1;
saturn_satp_tx_port:50100;
saturn_satp_tx_format:48000,float32_le,1,128;
saturn_satp_feedback:false;
```

`saturn_satp_enabled` reflects runtime configuration. The remaining fields
advertise capability and the configured destination even while the receiver is
disabled, so a client can explain why it is not sending.

The format tuple is:

```text
sample_rate_hz,sample_format,channels,frames_per_packet
```

The SATP destination host is the same host used for the TCI WebSocket. Only the
UDP port is announced. No command in this namespace grants operator or TX
authority.

## Compatibility rules

- Clients must ignore unknown `saturn_*` commands.
- Saturn ignores unknown client commands without disconnecting the client.
- SATP packet arrival never implies PTT.
- Standard TCI `trx` state and Saturn's existing safety state machine remain
  authoritative.
- TCI WebSocket binary audio is separate from SATP and remains available for
  existing Saturn Remote behavior.
