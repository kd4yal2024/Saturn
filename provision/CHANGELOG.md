# Changelog

All notable changes to provisioning assets are documented in this file.

## [2026-03-01]

### Added

- `cloud-init/saturn-provision-ui.cpp`
  - new C++/GTK3 desktop provisioning widget with:
    - stage/status display
    - elapsed and ETA countdown
    - toggleable live log panel
    - final success/failure summary based on provisioning state

### Changed

- `cloud-init/provision-saturn.sh`
  - added optional desktop UI launch controls:
    - `SATURN_DESKTOP_UI=auto|1|0`
    - `SATURN_UI_TIMEOUT_SECONDS`
    - `SATURN_UI_SHOW_LOG_DEFAULT`
    - `SATURN_UI_BINARY`
    - `SATURN_UI_STATUS_FILE`
  - installs desktop autostart for `SATURN_USER` (default `pi`) at `~/.config/autostart/saturn-provision-ui.desktop`
  - removes the desktop autostart entry automatically after successful provisioning
  - added UI status protocol file updates (`RUNNING|SUCCESS|FAILED`)
  - added explicit stage updates throughout provisioning for richer desktop progress feedback
  - enhanced error handling to publish failure state/messages for UI consumption
  - explicit runtime `SATURN_*` environment variables now override `/etc/default/saturn-provision` values

### Documentation

- `README.md`
  - documented optional desktop C++/GTK provisioning UI and usage flags
- `cloud-init/user-data.example.yaml`
  - added desktop provisioning UI example settings (`SATURN_DESKTOP_UI`, `SATURN_UI_TIMEOUT_SECONDS`, `SATURN_UI_SHOW_LOG_DEFAULT`)

## [2026-02-28]

### Changed

- `cloud-init/provision-saturn.sh`
  - run application/tool builds as `SATURN_USER` instead of root to avoid git safe-directory warnings
  - skip `P1_app` build in provisioning flow (not required for current images)
  - wait/retry every 5 minutes until `SATURN_USER` exists
  - make optional tool build failures non-fatal when `SATURN_BUILD_OPTIONAL_TOOLS=1`

### Documentation

- `README.md`
  - documented `P1_app` skip behavior and root-only cloud-init user-data access
  - documented `cloud-init clean --logs` before image capture and unique `instance-id` guidance
- `cloud-init/meta-data.example.yaml`
  - clarified `instance-id` should be unique per image/seed

## [2026-02-15]

### Added

- Cloud-init provisioning script:
  - `cloud-init/provision-saturn.sh`
- Cloud-init example files:
  - `cloud-init/user-data.example.yaml`
  - `cloud-init/meta-data.example.yaml`
- Provisioning documentation:
  - `README.md`
  - `CHANGELOG.md`

### Changed

- Provisioning flow now includes repo-clean protections for Python:
  - disables repo bytecode writes with `PYTHONDONTWRITEBYTECODE=1`
  - uses `PYTHONPYCACHEPREFIX=/var/cache/saturn-python`
  - blocks Python script execution from repo tree during provisioning
  - cleans `__pycache__`, `*.pyc`, `*.pyo` from repo before completion
