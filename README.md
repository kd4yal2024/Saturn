# Saturn

Saturn is an SDR appliance project for the Saturn/G2 radio platform. The repo
contains the FPGA assets, XDMA kernel driver, native Protocol 1/2/3 apps, Saturn
Go web manager, Saturn Bridge remote-control service, remote browser client,
provisioning flow, and supporting tools used to build and maintain a running
Saturn system.

The current operating model is appliance-first: a Raspberry Pi Compute Module
boots a provisioned image, loads the Saturn FPGA/XDMA path, runs the P2/P3 radio
app, and exposes maintenance plus remote operation through Saturn Go.

## Main Components

- `FPGA/` - Vivado project assets, IP, bitstreams, constraints, and FPGA
  support files. The current project still expects Vivado 2023.1 for rebuilds.
- `linuxdriver/xdma/` - active hardened XDMA kernel driver tree used by current
  Saturn images.
- `sw_projects/P1_app`, `sw_projects/P2_app`, `sw_projects/P3_app` - native C
  radio/protocol applications. `P2_app` is the current hardened Protocol 2 path
  used for Thetis/OpenHPSDR compatibility.
- `sw_tools/` - Saturn command-line tools, FPGA loader/flash tools, diagnostics,
  and `p2app-control` service integration.
- `scripts/` - host maintenance helpers, XDMA doctor/staging scripts, shutdown
  waiter, LCD/front-panel helpers, and update utilities.
- `install.sh` - canonical installer entry point for fresh or existing systems.
- `provision/` - cloud-init bootstrap that calls the same canonical installer.
- `update_manager/` - Saturn Go web manager, Saturn Bridge, remote web client,
  install/update scripts, web templates, and detailed docs.

## Recommended Entry Points

- Install a new system: `sudo ./install.sh`
- Cloud-init and image-factory flow: [`provision/README.md`](provision/README.md)
- Saturn Go architecture and operation:
  [`update_manager/README.md`](update_manager/README.md)
- Detailed Saturn Go docs:
  [`update_manager/docs/README.md`](update_manager/docs/README.md)
- Saturn Bridge remote backend:
  [`update_manager/saturn-bridge/README.md`](update_manager/saturn-bridge/README.md)
- Hardened P2 notes:
  [`sw_projects/P2_app/README.md`](sw_projects/P2_app/README.md)
- XDMA driver notes:
  [`linuxdriver/readme.txt`](linuxdriver/readme.txt)

## Provisioning

The appliance installer is designed for a Pi-based Saturn system. It installs
packages, kernel headers, XDMA support, Saturn apps/tools, Update Manager,
udev rules, the dedicated P2 runtime, Saturn Go, and Saturn Bridge with WDSP 2.00.

On a fresh Debian/Raspberry Pi OS Trixie arm64 installation:

```bash
cd ~
sudo apt-get update
sudo apt-get install -y git
mkdir -p github
cd github
git clone https://github.com/kd4yal2024/Saturn.git
cd Saturn
./install.sh --dry-run --user "$USER"
sudo ./install.sh --user "$USER"
```

Run `git clone` as the normal user from a writable directory. A first install
can take 30-60 minutes and needs stable power/network plus several gigabytes of
free space. Late in an interactive run, provisioning asks for the Saturn web
password: enter at least five characters, or press Enter to generate a
five-character value. This is separate from the Linux password. Unattended
installs generate it automatically; retrieve it locally with:

```bash
sudo cat /var/lib/saturn-provision/update-manager-admin-password
```

The checkout can otherwise live under any normal-user-owned directory. The
default `appliance` profile auto-detects the front panel and LCD, omits the
optional desktop developer/piHPSDR bundle, and uses hardware verification.
Optional profiles are:

- `sudo ./install.sh --profile desktop` for developer tools and piHPSDR
- `sudo ./install.sh --profile image-factory` for software-only verification

Cloud-init performs only first-boot/bootstrap work, checks out one requested
ref, and invokes this same installer non-interactively.

FPGA flashing is disabled by default and requires an explicit confirmation
variable when enabled.

XDMA is installed through DKMS. The older
`/etc/kernel/postinst.d/saturn-xdma` hook is automatically disabled whenever
the DKMS package is registered so kernel updates have one driver owner.

Interrupted installs resume completed package/build/deployment phases when the
same commit, kernel, and profile are used. Use `sudo ./install.sh --force` when
an intentional full reprovision is required.

Reboot after the final success message, then verify the XDMA devices, P2,
Saturn Go, Saturn Bridge, and web UI. `Front panel: NONE` is a valid result for
systems using a separate Waveshare LCD without a Saturn front panel. The full
operator checklist, cloud-init flow, LCD expectations, and failure recovery
commands are in [`provision/README.md`](provision/README.md).

## Saturn Go And Remote Operation

Saturn Go is the maintenance web UI and API. It provides:

- G2/Saturn update flows
- Saturn Go self-update
- piHPSDR/deskHPSDR update pages
- FPGA flash page
- backup/restore and Pi imaging tools
- process/system monitor
- XDMA Doctor
- Tailscale setup/status helpers
- Saturn Remote `/remote` and `/remote-next` entry points

Saturn Remote uses Saturn Bridge as the protocol boundary. The current preferred
remote operator path is `/remote-next` with split control/media WebSockets,
Opus TX, and the conservative ESSB CFC baseline documented in the
Saturn Go runbook.

## Security Notes

Saturn is powerful enough to key RF, flash FPGA images, restart services, and run
privileged maintenance helpers. Treat it as an appliance control plane.

- Saturn Go is intended to sit behind nginx basic auth on the LAN path.
- Saturn Remote TLS uses a separate `SATURN_REMOTE_BASIC_AUTH` service
  environment and fails closed when that auth is not configured.
- Tailscale is optional and should stack with Saturn Remote basic auth; it does
  not replace it.
- RF TX remains opt-in through bridge/service configuration.
- Newly set Saturn passwords are at least five characters with no composition
  rules; generated passwords are five characters. Existing credentials remain
  valid during upgrades. An
  unattended install generates a device-specific password and records it in
  `/var/lib/saturn-provision/update-manager-admin-password` (root-only).

## Build And Test

The modern Rust/TypeScript layers have automated tests:

```bash
cd update_manager/rust-server && cargo test
cd update_manager/saturn-bridge && cargo test
cd update_manager/remote-web && npm test
```

Native and kernel components are still primarily build/runtime validated:

```bash
make -C sw_projects/P2_app
make -C sw_projects/P2_app test-controller-lease
make -C sw_projects/P2_app cppcheck-ci
make -C sw_projects/P3_app
make -C linuxdriver/xdma
sudo bash scripts/install-xdma-dkms.sh --dry-run
```

The production Protocol 2 update path runs tests, builds, deploys through a
trusted root broker, verifies `p2app.service`, and rolls back on failure:

```bash
scripts/update-p2app.sh
```

## Golden Image

Install with the image-factory profile and complete bench verification before
sealing the source image. Close browsers and run the destructive sealing step
from a local console; it powers the source system off by default:

```bash
sudo ./install.sh --profile image-factory --verify hardware  # on a G2 bench
sudo scripts/seal-saturn-image.sh --confirm SEAL
```

Use the image-factory default (software verification) when preparing the image
off-hardware; run the explicit hardware verification form above on a G2 bench.

Sealing powers the source system off after removing machine, SSH, Tailscale,
Saturn Remote TLS/cookie, and administrator identity. Each clone generates a
unique hostname (unless customized), five-character Saturn Go login, and
Remote TLS certificate on first boot.
A Linux password supplied by Raspberry Pi Imager or cloud-init is preserved. If
the local Saturn account is still locked, first boot unlocks it with the same
generated value for simple initial access. The file records which case applied
and lists the separate commands to change each credential. Retrieve it locally
with:

```bash
cat /var/lib/saturn-state/initial-login.txt
```

The sealer removes the builder's Wi-Fi/cloud-init network seed. Configure the
recipient's network with Raspberry Pi Imager, a new cloud-init seed, or wired
Ethernet DHCP. Keep the provisioned Saturn username unchanged when customizing
a sealed image; the installed services intentionally run as that account.

The GitHub Actions workflow under `.github/workflows/ci.yml` is the first
baseline CI gate for these checks. ShellCheck is installed by CI as a
development-only lint dependency; it is not installed on operator appliances
by `sudo ./install.sh`.

## XDMA Driver Policy

The supported XDMA driver is `linuxdriver/xdma`.

For field recovery, `scripts/fix-xdma.sh` remains the direct rebuild/install
helper. For beta systems moving to standard kernel upgrade handling,
`scripts/install-xdma-dkms.sh` stages the same supported source as a DKMS
package named `saturn-xdma`. A successful DKMS install disables the legacy
manual kernel postinst hook to avoid duplicate XDMA rebuilds during kernel
package updates. DKMS versions are derived from the driver source; older
versions remain registered until an operator explicitly enables pruning after
the replacement is installed.

The old `linuxdriver/xdma_pre_kernel_5.18` source tree has been retired for beta
because it drifted behind the active hardened driver. If an old-kernel recovery
case needs it, use git history and backport the active-tree safety fixes before
shipping it.

## Repository Notes

This repository intentionally includes release FPGA bitstreams and hardware
support assets. That makes the checkout larger than a pure source repository,
but keeps the Saturn image/provisioning path self-contained.
