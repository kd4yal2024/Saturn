# Saturn Update Manager Documentation

This folder contains the operational and technical documentation for the Rust-based Saturn Update Manager (`saturn-go`).

## Read This First

1. `ARCHITECTURE.md`
2. `FEATURE_MATRIX.md`
3. `API_REFERENCE.md`
4. `SCRIPT_CATALOG.md`
5. `OPERATIONS_RUNBOOK.md`
6. `RUST_SUPPLY_CHAIN.md`
7. `ADMIN_AUTH_SIMPLIFICATION.md`
8. `XDMA_RELEASE_POLICY.md`
9. `REMEDIATION_PLAN.md`
10. `RELEASE_MANIFEST_SCHEMA.md`
11. `STATE_COMPATIBILITY.md`
12. `SATURN_REMOTE_ARCHITECTURE.md`
13. `SATURN_REMOTE_STATE_STRIP_SPEC.md`
14. `SATURN_REMOTE_APPLE_SAFARI_VALIDATION.md`
15. `SATURN_REMOTE_APPLE_SAFARI_RESULTS.md`
16. `SATURN_REMOTE_SQUELCH_AUDIO_PROFILE_CONTRACT.md`
17. `SATURN_REMOTE_TX_UPLINK_PHASE38_CONTRACT.md`

## Document Guide

- `ARCHITECTURE.md`
  - How requests flow through NGINX, the Rust API, scripts, and system services.
  - Runtime paths, persisted state, and security model.

- `FEATURE_MATRIX.md`
  - One place that maps each major feature to UI page, API endpoints, scripts, and state files.

- `API_REFERENCE.md`
  - Endpoint-by-endpoint reference for the backend API.
  - CSRF requirements, request format, and response notes.
  - Includes `/run_log` terminal resume/buffer endpoint details.

- `SCRIPT_CATALOG.md`
  - Inventory of deployed scripts, versions, flags, and which API/UI path calls each one.

- `OPERATIONS_RUNBOOK.md`
  - Build, install, uninstall, daily operations, and troubleshooting.
  - Includes the Overview and Radio Telemetry operator views, G2 Update page
    (G2 + Appliance Update), dedicated application/FPGA update flows,
    browser-managed custom scripts, backup/restore, and service checks.

- `RUST_SUPPLY_CHAIN.md`
  - Rust dependency policy for `saturn-go` and `saturn-bridge`.
  - Documents `cargo audit`, `cargo deny`, lockfile, and pinning expectations.

- `ADMIN_AUTH_SIMPLIFICATION.md`
  - Admin password design for the ham-operator audience: single source of truth
    (`saturn-admin-password.sh`), console-only reset, and the phased plan for
    remember-this-device cookies and passwordless Tailscale identity auth.

- `XDMA_RELEASE_POLICY.md`
  - DKMS package versioning, manual postinst-hook takeover, and rollback policy.

- `REMEDIATION_PLAN.md`
  - Trackable reliability milestones, approved appliance-usability decisions,
    validation gates, and completion evidence.

- `RELEASE_MANIFEST_SCHEMA.md`
  - Immutable application bundle identity, inventory, validation, and
    persistent-state compatibility fields.

- `STATE_COMPATIBILITY.md`
  - Persistent-state versioning, migration backups, rollback behavior, and
    the explicit one-way-migration policy.

- `SATURN_REMOTE_ARCHITECTURE.md`
  - Browser, TLS, bridge, TCI, audio, panadapter, and waterfall architecture
    for the Saturn Remote operator console.

- `SATURN_REMOTE_STATE_STRIP_SPEC.md`
  - `/remote-next` operator state strip contract: pill order, labels, density rules, escalation rules, and acceptance criteria.

- `SATURN_REMOTE_APPLE_SAFARI_VALIDATION.md`
  - Manual validation runbook for `/remote-next` on macOS Safari, iPhone Safari, and iPad Safari.
  - Covers `Go Live` audio unlock, touch/hold PTT release, screen lock/background fail-closed behavior, WebGL display checks, and settings persistence.

- `SATURN_REMOTE_APPLE_SAFARI_RESULTS.md`
  - Evidence ledger for the real Apple Safari validation rows.
  - Tracks whether Mac Safari, iPhone Safari, and iPad Safari are pending, passed, failed, or blocked.

- `SATURN_REMOTE_SQUELCH_AUDIO_PROFILE_CONTRACT.md`
  - Pre-implementation bridge/UI contract for RX squelch and quick audio profiles on `/remote-next`.
  - Defines the first SSB syllabic squelch slice, TCI command shape, WDSP mapping, UI placement, and acceptance criteria.

- `SATURN_REMOTE_TX_UPLINK_PHASE38_CONTRACT.md`
  - Pre-implementation contract for slow-VPN TX uplink safety on `/remote-next`.
  - Defines browser `bufferedAmount` guard thresholds, bridge-authoritative `tx_fault:uplink_late`, bidirectional TX mic telemetry, regression tests, and acceptance criteria.

## Engineering History

- `SATURN_REMOTE_PHASE36_MEASUREMENT.md`
  - Measurement notes and evidence from the Phase 36 Remote performance work.
- `SATURN_REMOTE_PHASE37_ANALYSIS.md`
  - Analysis and decisions from the Phase 37 Remote transport work.
