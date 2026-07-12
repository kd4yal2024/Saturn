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
- `scripts/install-saturn-appliance.sh` - complete installer for fresh or existing systems.
- `provision/` - legacy cloud-init wrapper for image-based first boot.
- `update_manager/` - Saturn Go web manager, Saturn Bridge, remote web client,
  install/update scripts, web templates, and detailed docs.

## Recommended Entry Points

- Install a new system: `sudo scripts/install-saturn-appliance.sh`
- Legacy cloud-init flow: [`provision/README.md`](provision/README.md)
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

Run it from a checked-out repository; cloud-init is not required:

```bash
sudo scripts/install-saturn-appliance.sh
```

The cloud-init examples remain available for image factories and invoke the
same supported WDSP 2.00 bridge path.

FPGA flashing is disabled by default and requires an explicit confirmation
variable when enabled.

Provisioning and the Saturn Go installer also install the XDMA kernel
post-install hook at `/etc/kernel/postinst.d/saturn-xdma`. That hook pre-stages
`xdma.ko` for newly installed kernels without unloading the live module or
restarting `p2app.service`.

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
remote operator path is `/remote-next` with Phase 42 split control/media sockets,
Phase 44 Opus TX, and the conservative ESSB CFC baseline documented in the
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
- Beta provisioning may intentionally keep `admin/admin` for first access. That
  is a beta convenience only and should be changed before untrusted network use.

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

The GitHub Actions workflow under `.github/workflows/ci.yml` is the first
baseline CI gate for these checks.

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
