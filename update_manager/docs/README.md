# Saturn Update Manager Documentation

This folder contains the operational and technical documentation for the Rust-based Saturn Update Manager (`saturn-go`).

## Read This First

1. `ARCHITECTURE.md`
2. `FEATURE_MATRIX.md`
3. `API_REFERENCE.md`
4. `SCRIPT_CATALOG.md`
5. `OPERATIONS_RUNBOOK.md`
6. `SATURN_REMOTE_STATE_STRIP_SPEC.md`
7. `SATURN_REMOTE_APPLE_SAFARI_VALIDATION.md`
8. `SATURN_REMOTE_APPLE_SAFARI_RESULTS.md`

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
  - Includes G2 Update page (G2 + Appliance Update), dedicated piHPSDR update flow, dedicated FPGA flash flow, browser-managed custom scripts flow, backup/restore flow, and service checks.

- `SATURN_REMOTE_STATE_STRIP_SPEC.md`
  - `/remote-next` operator state strip contract: pill order, labels, density rules, escalation rules, and acceptance criteria.

- `SATURN_REMOTE_APPLE_SAFARI_VALIDATION.md`
  - Manual validation runbook for `/remote-next` on macOS Safari, iPhone Safari, and iPad Safari.
  - Covers `Go Live` audio unlock, touch/hold PTT release, screen lock/background fail-closed behavior, WebGL display checks, and settings persistence.

- `SATURN_REMOTE_APPLE_SAFARI_RESULTS.md`
  - Evidence ledger for the real Apple Safari validation rows.
  - Tracks whether Mac Safari, iPhone Safari, and iPad Safari are pending, passed, failed, or blocked.
