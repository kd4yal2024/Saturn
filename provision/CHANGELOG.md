# Changelog

All notable changes to provisioning assets are documented in this file.

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
