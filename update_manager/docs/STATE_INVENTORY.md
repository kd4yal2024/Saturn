# Saturn Persistent-State Inventory

This document is the REM-0301 inventory of data that can affect a Saturn
appliance. The machine-readable source of truth is
`update_manager/release/state-inventory-v1.json`.

This inventory is consumed by the separate REM-0302 settings/release backup
formats and the REM-0303 transactional restore paths.

## Classification rules

| Portability | Meaning |
|---|---|
| `portable` | May move between compatible Saturn appliances after schema validation. |
| `review-before-transfer` | Operator data, but it contains hardware, radio, path, or transmit choices that must be reviewed on a different appliance. |
| `same-device-only` / `device-specific` | Identity, credential, topology, boot, or recovery data that must not be cloned to another radio. |
| `regenerable` | Rebuild, reinstall, or fetch it from its authoritative source. |
| `external` | Owned outside the Saturn settings system or stored in hardware. |
| `diagnostic` | Useful for troubleshooting, not for restoration. |

`secret` means the file contents must never be placed in an ordinary support
bundle. `sensitive` means a support bundle must include only the stated
metadata or sanitized content. Paths in the JSON inventory use
`${SATURN_HOME}`, `${SATURN_REPO}`, `${PIHPSDR_REPO}`, and
`${DESKHPSDR_REPO}` because those locations are configurable.

## Irreplaceable operator settings

These are the planned inputs to the small settings backup in REM-0302:

| Data | Location | Transfer rule | Notes |
|---|---|---|---|
| Saturn Remote current settings | `/var/lib/saturn-state/remote_settings.json` | Portable | Includes RX/DSP/display settings and TX drive, mic gain, EQ/CFC, filters, ADC, and antenna choices. Review TX values on another radio. |
| Saturn Remote named profiles | `/var/lib/saturn-state/remote_profiles.json` | Portable | Same radio/transmit review as current settings. |
| State schema marker | `/var/lib/saturn-state/state-schema.json` | Portable and required | Must accompany managed settings so compatibility can be checked. |
| Custom-script registry | `/var/lib/saturn-state/custom_scripts.json` | Portable | Restore with the selected operator-authored files. |
| Operator custom scripts | selected files in `/opt/saturn-go/scripts` | Review before transfer | Packaged scripts are not backed up. Script content may contain secrets or destructive commands. |
| piHPSDR properties | `${PIHPSDR_REPO}/*.props` | Review before transfer | These files may include protocol, GPIO, radio, PA/calibration, antenna, and drive choices. |
| deskHPSDR properties | `${SATURN_HOME}/.config/deskhpsdr/*.props` | Review before transfer | Logs in this directory are not settings. Review XDMA/network selection and all radio/calibration values. |
| Update policy | `repo_root.txt`, `update_policy.json`, `saturngo_update_policy.json` | Review before transfer | Repository paths are host-specific even when public owner/repository/ref policy is reusable. |

No separate P2 application calibration database was found. Calibration-like
operator data currently belongs to Saturn Remote, piHPSDR, or deskHPSDR
properties. The FPGA image actually programmed into the radio is hardware
state; the repository `FPGA/` directory is not proof of what is flashed.
Record the reported FPGA version/date and chosen artifact checksum. The FPGA
maintainers remain the owner of a future authoritative artifact/readback
policy.

## Same-device disaster-recovery state

This data matters when recovering the **same** appliance, but it is unsafe or
incorrect to move blindly to another radio:

- Saturn administrator credential backends, initial-login recovery file, and
  Linux password hashes.
- Remembered-device HMAC secret. Browser cookies are stored in each browser,
  not on Saturn. Omitting this secret deliberately signs every remembered
  browser out.
- Saturn Remote TLS certificate/private key.
- Tailscale node state and NetworkManager/Wi-Fi credentials.
- Machine ID, hostname mapping, and SSH host keys.
- `/boot/firmware/config.txt` and `cmdline.txt` (or legacy `/boot` paths),
  including LCD, UART/front-panel, I2C, USB, and GPIO choices.
- Provisioned hardware profile, front-panel type, system role, phase/image
  markers, and local service policy.
- Release deployment transaction, rollback snapshot, and state-migration
  backups. These records describe one host and must not be transplanted.

For a replacement or cloned appliance, rerun first-boot/provisioning, create a
new administrator password, generate new TLS/SSH/machine identity, and enroll
Tailscale as a new node. Then import only the portable settings after review.

## Regenerable and diagnostic data

The following is not part of a small settings backup:

- Saturn, piHPSDR, and deskHPSDR source repositories, except for explicit
  uncommitted operator work and their property files.
- `/opt/saturn/releases`, build outputs, dependency trees, staged worktrees,
  snapshots, and compiler caches. Exact releases should be rebuilt or
  reinstalled from a validated release artifact.
- Installed systemd units, udev rules, sudoers files, web assets, binaries, and
  packaged scripts, which the canonical installer recreates.
- Journald, provisioning logs, update logs, deskHPSDR logs, and volatile
  `/dev/shm` telemetry. These are diagnostic evidence only.
- The repository `FPGA/` tree. It belongs with source/release artifacts, while
  flashed FPGA state is external hardware state.

## Current backup behavior

The names shown in the existing UI are historical and narrower than they
sound:

| Existing operation | Exactly contains | Explicitly omits |
|---|---|---|
| `GET /backup_settings` | A versioned, checksummed selection of Saturn Remote settings/profiles, state schema, custom-script registry and registered operator scripts, update policy, and piHPSDR/deskHPSDR `*.props`. | Credentials, cookie/TLS/SSH secrets, Tailscale/network/host identity, boot and provisioning config, deployments/logs, source/releases/caches, and FPGA hardware state. |
| `GET /backup_source` (`GET /backup_full` compatibility alias) | A gzip tar archive of the current Saturn **repository root**. | Everything outside that repository: administrator/Linux credentials, remembered-device/TLS identity, Saturn Remote settings/profiles, custom-script registry and installed custom files, piHPSDR/deskHPSDR settings outside that repository, Tailscale/network state, boot/LCD/front-panel config, provisioning state, deployment history, installed releases, and FPGA hardware state. It is not a full appliance backup. |
| `GET /backup_release?commit=...` | One manifest-bearing immutable release directory for the selected full commit. | Settings, credentials, source repositories, OS/network/boot state, deployment history, other releases, and FPGA hardware state. |
| `saturn-backup-*` made by `update-G2.py` | A directory copy of the Saturn source repository at update time. | The same appliance state omitted by `/backup_full`. |
| `pihpsdr-backup-*` made by `update-pihpsdr.py` | A directory copy of `${PIHPSDR_REPO}`. This normally includes `.props` files stored at that repository root. | Saturn state, deskHPSDR properties, operating-system identity/configuration, and any piHPSDR data stored outside the repository. |
| `deskhpsdr-backup-*` made by `update-deskhpsdr.py` | A directory copy of `${DESKHPSDR_REPO}`. | `${SATURN_HOME}/.config/deskhpsdr/*.props`, Saturn state, and operating-system identity/configuration. It is a source rollback, not a radio-settings backup. |
| REM-0205 pre-migration backup | Only the declared direct Saturn state files plus the state-schema marker, with checksums/modes/owners, under `deployments/state-backups`. | Credentials, TLS/cookies, custom script content, client-app properties, networking, boot config, release payloads, and all undeclared files. It exists only to roll back an activation migration. |
| Manual whole-disk image | Every allocated/read block captured from the selected source device, subject to the imaging tool and consistency of the live filesystem. | External browser cookies, the current contents of FPGA flash unless separately read, external credentials/services, and anything on another disk. Disk imaging is local-console-only and remains outside Saturn Go. |

Operators must not use “full backup” to mean “recoverable appliance.” The
legacy `/backup_full` endpoint is only a compatibility alias for the source
repository archive. `BACKUP_FORMATS.md` is the complete REM-0302 contract.

## Ordinary support-bundle policy

There is no automatic support-bundle exporter yet. Any manually assembled
bundle must follow this allowlist/exclusion policy.

Allowed after review:

- build commit, release manifest/checksums, active release pointer, package and
  kernel versions;
- provisioning hardware profile and boot configuration;
- deployment/update status and bounded logs;
- public TLS certificate fingerprint/expiry and public SSH host fingerprints;
- filenames, sizes, modes, hashes, and schema versions for settings/custom
  scripts—never their content by default;
- sanitized radio telemetry and service status.

Always excluded:

- administrator plaintext/hash backends and `/etc/shadow`/`gshadow`;
- `initial-login.txt` and any `SATURN_ADMIN_PASSWORD` value;
- remembered-device secret, TLS private key, SSH private keys, machine ID;
- Tailscale state, NetworkManager/Wi-Fi secrets, GitHub OAuth state, Git/SSH,
  npm, Cargo, and `.netrc` credentials;
- custom-script content;
- raw radio profiles/property files unless the operator explicitly inspects
  and attaches selected files for a specific support case.

Logs and JSON status may still contain user-entered arguments, repository URLs,
hostnames, addresses, call signs, or radio configuration. Sanitize them even
when their inventory entry permits support use.

## Open ownership boundaries

- REM-0303 validates the REM-0302 settings manifest, exact selection, and
  review-before-transfer host policy before import.
- REM-0303 provides crash-recoverable settings staging and atomic source
  generation activation; piHPSDR's separate legacy source restore is outside
  this inventory's Saturn state boundary.
- FPGA maintainers must define authoritative artifact naming, signing/hash,
  compatibility, and whether verified hardware readback is possible.
- piHPSDR and deskHPSDR remain owners of their property-file schemas. Saturn
  can preserve those files but cannot promise cross-version compatibility.
