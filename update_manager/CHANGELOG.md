# Changelog

All notable changes to the Saturn Update Manager (Rust) are documented here.

## [Unreleased]
### Changed
- Release builds can now select a remote branch or tag, resolve it to one
  immutable commit, detect movement before staging, and build from a detached
  worktree. Schema-v3 manifests record the source remote, requested ref,
  canonical resolved ref, and exact tested commit; v1/v2 manifests remain
  readable for rollback.
- Made the one-year Saturn Remote remembered-login policy an explicit tested
  contract. Administrator password changes continue to invalidate every
  remembered browser after the deferred Saturn Go restart, with clearer UI
  messaging and shared-device risk/recovery guidance for operators.
- Restart Saturn Bridge after replacing its executable during standalone or
  appliance installation. Previously, `systemctl enable --now` left an
  already-running service on the deleted prior binary until a reboot or manual
  restart.
- Limited Saturn Remote to four authenticated logical clients (a paired split
  control/media session counts once) with clean HTTP admission failures and an
  eight-socket bridge backstop. Inbound commands and per-client outbound
  control are now bounded/coalesced; microphone and display traffic retain
  only their newest bounded data, while TX release and safety commands use the
  priority lane. `/remote_metrics`, `/bridge_diag`, bridge logs, and
  `remote_backpressure` now expose connection, queue, drop, latency, and
  high-water metrics.
- Bounded maintenance-script live output to 128 queued events, in-memory resume
  output to 1 MiB/5,000 lines, and durable output to 4 MiB/5,000 lines with
  explicit backpressure/truncation notices. Routine scripts now default to a
  30-minute deadline, update scripts to four hours, and caller overrides are
  clamped to a six-hour absolute maximum. Tailscale helpers use the same
  bounded live-output channel and a ten-minute deadline.
- Replaced the global 2 GiB request allowance with 64 KiB ordinary and
  custom-script limits plus a separate configurable, streamed restore limit.
  Restore upload/extraction now preserves the readiness disk reserve, and
  stages on the persistent Saturn state filesystem rather than the small
  `/tmp` tmpfs. Oversized ordinary requests fail early with HTTP 413.

### Fixed
- Provisioning and direct udev/P2 installers now add the configured desktop
  operator to `saturn-radio`. This preserves group-restricted XDMA device
  permissions while allowing locally launched piHPSDR and deskHPSDR to open
  `/dev/xdma0_user` after a new login or reboot.

### Added
- Added an explicit deskHPSDR legacy GPIO V1 channel for Debian Trixie. The
  updater pins upstream `2.6.84`, applies Saturn's libgpiod-v2 port, requires
  the legacy controller source, verifies GPIO is present in the built binary,
  and can return the checkout to the current upstream channel on the next
  normal run.
- Added a built-in Saturn Remote **Voodoo 3.8k** TX audio profile with a true
  50-3850 Hz passband, warm restrained EQ/CFC curves, microphone headroom, and
  one-click application that never changes drive, tuning, PTT/MOX, or RF state.
- Added the first operational direct-XDMA receive backend behind the
  production-disabled backend selector. DDC6/ADC1 IQ now feeds the existing TCI
  and WDSP receive paths at 192 kHz, live tuning and RX DSP controls remain
  available, all TX requests fail closed, and signal-driven shutdown restores
  the receive-safe register state. The transaction broker now requires a fresh
  operational heartbeat with DMA and IQ counters before accepting XDMA, while
  the installed policy continues to block production selection pending
  appliance validation. A bounded source-tree smoke harness rejects stale
  binaries, proves a steady-state IQ rate within two percent of 192 kHz plus
  receive-safe shutdown, and restores the prior radio-service activity without
  changing that policy. The first appliance run sustained approximately
  191,975 IQ pairs/second with no framing or FIFO fault.
- Added a bounded full backend-transaction acceptance that stages the tested
  bridge under `/opt/saturn-go` for the hardened systemd namespace and the
  appliance's `noexec` `/run`, proves exclusive `P2 -> XDMA -> P2` ownership
  through the production Saturn Go split proxy, restores the exact prior
  appliance state, and emits focused diagnostics on any failed transition. The
  production XDMA selector remains disabled.
- Added a dependency-free direct-XDMA TCI acceptance client. It validates live
  IQ and receive-audio frames, continued streaming after a DSP preference
  burst, retuning, fail-closed TX refusal, disconnect, and service restoration.
  Client testing exposed and corrected command-burst DDC starvation by making
  the operational loop RX-first, bounding each control slice, synchronizing
  WDSP only for DSP changes, and replacing complete-state floods with targeted
  acknowledgements. The passing debug-build run sustained 191,992 IQ
  pairs/second with an 8,016-word FIFO high-water and zero FIFO/framing faults.
- Extended direct-XDMA acceptance through Saturn Go's authenticated TLS split
  control/media proxy, including session pairing without exposing the stored
  credential. Added a bounded appliance transaction harness for test-gated
  `P2 -> XDMA -> P2` validation with exact pre-test restoration, plus regression
  coverage for the round trip and rollback from a failed return to P2.
- Added opt-in XDMA completion diagnostics for the direct-DMA Phase 4 stress
  work. Slow synchronous transfers now separate page mapping, submission wait,
  and cleanup; the submission wait further reports submit-to-IRQ,
  IRQ-to-worker, worker-to-wake, and wake-to-caller latency.
- Added an opt-in event-driven SCHED_FIFO XDMA completion thread after staged
  latency diagnostics isolated network-stress outliers in deferred workqueue
  dispatch rather than hardware DMA, IRQ delivery, or caller wakeup. Saturn
  appliances now persist the validated priority-20 completion policy and 5 ms
  rate-limited diagnostic threshold across module reloads and reboots.
- Moved Phase 4 progress output off the real-time refill thread, made automatic
  CPU selection the stress-harness default, and widened the DUC FIFO operating
  band to an efficient eight-frame window. The validated band halves refill
  calls while absorbing rare scheduler stalls under combined appliance load.
  Stress runs now fail before applying load unless the loaded driver has the
  validated priority-20 completion thread, with an explicit A/B-only override.
  The final 30-minute combined stress gate sustained 192 kHz with zero
  critical-low or runtime FIFO-fault events, 0.600 ms p99.99 refill service,
  and 1.632 ms maximum refill service.
- Added the first Phase 5 guarded-transmit preflight. It never keys RF, expands
  common emergency cleanup to zero and disable the entire transmit data path,
  verifies receive-safe register readback, and provides bounded failure
  checkpoints for testing automatic cleanup.
- Added the first explicitly authorized Phase 5 RF probe, locked to PCB2
  firmware 1.27, ANT1, 7.200 MHz, and a 3 W ceiling. It preloads the validated
  direct-XDMA DUC FIFO while RF is inhibited, ramps only to drive byte 11,
  continuously trips on FIFO, forward/reverse-power, or SWR faults, bounds
  key-down to 500 ms, and returns the full transmit path to receive-safe state
  before reporting results. The initial antenna-connected field run produced
  1.403 W forward, zero measured reverse power, no FIFO fault, and verified
  cleanup.
- Split Saturn Go health reporting into `/livez` process liveness and
  structured `/readyz` dependency/release readiness. Rust builds embed their
  full Git commit; staged Saturn Go payloads carry the same identity, and the
  root deployment broker verifies that exact commit after restart before
  accepting a deployment. `/healthz` remains a temporary liveness alias.
- The Overview page now reports Saturn Go readiness, the running commit, and
  any failed required readiness components. The watchdog uses liveness while
  installation and deployment use target-aware readiness.
- Canonical provisioning now defers its nested Saturn Go readiness probe until
  Saturn Bridge has been installed, then verifies the exact target commit. This
  prevents a clean appliance install from failing merely because Bridge is
  intentionally installed in the following orchestration step.
- Added the Milestone 2 inactive application-release builder, v1 component
  policy, structured release manifest, and exact payload checksum validation.
  Release construction runs the normal non-hardware gates and never changes
  the active release pointer.
- Added the root-owned immutable release installer. It revalidates staged
  manifests and hashes, rejects unsafe entries and mismatched architectures,
  copies through a private sibling directory, installs root-owned releases at
  `/opt/saturn/releases/<full-commit>`, and never activates or restarts them.
- Preserve manifest-declared executable modes below the release staging root
  when the canonical installer hardens mutable Saturn state permissions.
- Normalize bundle permissions before manifest creation and reject any
  group/world-writable payload directly in manifest creation and validation.
- Require every immutable release directory to use mode `0755` so services can
  read root-owned releases without making them writable.
- Added the REM-0203 root-owned release activation transaction. It persists a
  prepared transaction, atomically changes the stable release pointer, wires
  Saturn Go, Bridge, and P2 through systemd drop-ins, restarts them in defined
  dependency order, and commits only after exact-commit readiness succeeds.
  Production activation remains disabled by default pending a separately
  approved live appliance test.
- Added REM-0204 automatic activation rollback. The transaction snapshots the
  prior pointer, exact ready commit, service activity, and systemd overrides;
  failures restore and restart the prior deployment and require its exact
  commit to become ready. Durable state distinguishes `rolled_back` from
  `rollback_failed`, and activation never prunes installed releases.
- Added REM-0205 persistent-state compatibility contracts to schema-v2 release
  manifests. Activation now preflights state readability, creates a
  checksummed backup before migration, restores that state during automatic
  rollback, and blocks undocumented or unapproved one-way migrations. Legacy
  schema-v1 releases map to state schema 0. Production activation remains
  disabled by default.
- Added the REM-0301 persistent-state inventory and automated policy contract.
  It classifies Saturn credentials, remembered-device/TLS identity, Remote and
  client-radio settings, calibration-bearing property files, custom scripts,
  networking/Tailscale, boot/LCD/front-panel configuration, deployment state,
  release/source artifacts, FPGA hardware boundaries, and diagnostics by
  portability, sensitivity, recovery importance, and support-bundle handling.
  Backup documentation now states exactly what every existing repository,
  migration, and manual whole-disk backup contains and omits. No backup,
  restore, support-bundle, activation, or live-host behavior changed.
- Added REM-0302 separated backup types. The new settings download stages only
  managed Saturn settings, registered operator scripts, and direct
  piHPSDR/deskHPSDR property files into a bounded schema-v1 archive with
  per-file SHA-256 metadata and explicit credential/device-identity
  exclusions. Source backup now has an accurate endpoint/name, while the old
  `/backup_full` URL is only a compatibility alias. Installed immutable
  releases can be listed and exported one validated full commit at a time.
  The Backup page and runbook clearly separate settings, source, release, and
  manual whole-disk disaster recovery, and label existing repository restore
  as legacy/nontransactional. Settings and release import remain disabled for
  REM-0303; live release activation is unchanged.
- Added REM-0404 graceful maintenance shutdown. API and signal-driven stops
  close job admission before draining active work; transactional and unknown
  work finishes, while explicitly cancel-safe cleanup/report scripts receive
  process-group TERM/KILL and persist a `cancelled` terminal result. The
  generated systemd service now signals the controller first with
  `KillMode=mixed` and allows a bounded finish window.
- Unified provisioning entry point (`./install.sh`) shared by manual Trixie
  installs and the cloud-init bootstrap. The appliance engine now supports
  appliance/desktop/image-factory profiles, bounded user discovery, exact-ref
  bootstrap checkout, resumable expensive phases, versioned host state, and
  separate software/hardware verification.
- Golden-image sealing and first-boot personalization. A sealed clone receives
  unique machine/SSH, hostname, and Saturn Remote TLS identity plus a unique
  five-character Saturn Go login instead of inheriting credentials,
  certificates, cookies, login hashes, builder keys/logs, or Tailscale state
  from the source image. First boot preserves a Linux password supplied by
  image customization and only assigns the generated value when the local
  account remains locked.
- Production Protocol 2 test/build/deploy workflow with a root-owned fixed-path
  broker, service health verification, and automatic binary rollback.
- Newly set Saturn administrator passwords are at least five characters with no
  composition rules. Generated passwords remain five characters, and existing
  synchronized credentials remain active during upgrades.
- Initial installation and later password changes now use the same
  transactional credential helper, so nginx and Saturn Remote auth are never
  written by separate provisioning implementations.
- XDMA installation now has one lifecycle owner: canonical provisioning uses
  DKMS and all nested installers keep the legacy kernel post-install hook
  disabled whenever the DKMS package is registered.
- Saturn Remote `/remote-next` now automatically recovers from bridge restarts,
  split control/media lane failures, and browser network interruptions. A
  single session supervisor uses bounded exponential backoff with jitter,
  socket-connect and bridge-ready watchdogs, stale-attempt rejection, and
  browser online/offline awareness. Recovery replays frequency, preferences,
  IQ, and RX audio only after the bridge reports ready, while TX/PTT/microphone
  state remains fail-closed. Connection status, retry attempts, reasons, and
  recovery timing are visible in Operator State and copied RX diagnostics.
- Update G2 logging now survives an unwritable `~/saturn-logs` directory by
  warning and using a private temporary log. The installer repairs the primary
  log directory ownership for the configured Saturn Go service user and passes
  the resolved path to the service environment.
- Saturn Go now has a shared offline appliance shell across Overview, Monitor,
  update, FPGA, backup, application-update, Tailscale, and custom-script pages.
  It provides grouped sidebar navigation, an accessible mobile drawer, common
  theme/design tokens, consistent controls/status styling, and locally served
  Inter, Tailwind, Chart.js, and ansi_up assets. Prefix-safe asset/navigation
  URLs work through production's `/saturn/` proxy as well as direct backend
  routes.
- A new Overview landing page summarizes system utilization, radio/FPGA state,
  network/Tailscale activity, service health, and the last Saturn Go deployment
  with quick access to the main operator and maintenance workflows. Overview
  rate sampling uses an isolated backend scope so it does not disturb Monitor
  chart rates.
- Saturn Remote stable and next-generation pages now use the same locally
  hosted Inter font and provide working Saturn Go/Monitor navigation back to
  the HTTP management console, including IPv6 host handling.
- The former hidden P23 service lab is now presented as Radio Telemetry &
  Diagnostics, linked from the primary navigation at `/telemetry`, while the
  existing `/p23test` routes remain available for compatibility.
- Shared management-page headers and both Saturn Remote variants now use the
  same compact `Saturn G2` title eyebrow for consistent appliance branding.
- The system navigation and page heading now identify Tailscale as `Tailscale
  VPN`. Radio Telemetry Bridge Diagnostics now constrains both grid columns and
  wraps long journal/JSON lines so the service summary remains readable.
- The Radio Telemetry performance dashboard now has an appliance-style,
  activity-aware layout with primary CPU/DDC/DUC/XDMA KPIs, compact radio,
  system, and reliability sections, and explicit active, idle, degraded, and
  fault states. Idle radios display as healthy and waiting instead of showing
  misleading zero-rate KPIs or XDMA-drop alerts. Telemetry read/parse failures
  are reported directly, including explicit disabled, waiting, live, stale,
  unreadable, and invalid states for optional ADC peak telemetry. The dashboard
  also surfaces ADC dBFS, DMA operation efficiency, queue context, FPGA/SoC
  temperature, CPU frequency, clock state, and fallback-image state. A directly
  installed `p2app.service` workload is identified from its running command
  line when no deployment-slot symlink exists.
- Reproducible non-cloud-init Saturn Bridge deployment: the Saturn Go installer
  now enables the bridge by default, provisions sparse upstream sources at
  pinned commits, builds/verifies WDSP 2.00 and the bridge, and installs the
  service with the matching Remote UI assets. Saturn Go self-update now stages
  the bridge binary and unit alongside the backend and web bundle.
- Saturn Bridge PureSignal 3.0 support using synchronized 192 kHz DDC0 ADC
  feedback and DDC1 TX-DAC reference samples, WDSP automatic calibration and
  correction, automatic/manual feedback attenuation, feedback-loss bypass,
  and live state telemetry in Saturn Remote Setup -> TX. Saturn Remote's
  Operator Log now records coalesced calibration/state/attenuation events,
  periodic active feedback samples, packet-gap faults, and a complete
  PureSignal snapshot in copied log exports.
- Saturn Remote WDSP 2.00 controls for NR2 gain/noise-estimation methods,
  psychoacoustic post filtering, and the TX phase rotator with manual or
  automatic corner-frequency operation. The bridge persists and publishes the
  new TCI state, disables phase rotation in coherent digital modes, and
  feature-detects the WDSP 2.00 auto optimizer for legacy-library compatibility.
- WDSP 2.00 wideband FM stereo reception with a 192 kHz RX DSP path, stereo
  lock indication, selectable North American 75 us or European 50 us
  de-emphasis, FM broadcast band memory, and receive-only safeguards. The
  Remote UI also has a denser operator layout that keeps the spectrum,
  waterfall, VFO, and primary receive controls visible together on desktop
  while preserving a spectrum-first phone layout.
- New `/remote-next` page served from the Saturn Remote TLS listener as the
  next-generation Saturn Remote UI, alongside the existing stable `/remote`
  page. The two pages share the same basic-auth gate, persisted
  `remote_settings.json` / `remote_profiles.json` state, and `saturn-bridge`
  TCI websocket; `/remote-next` carries the active extraction work while
  `/remote` remains the stable fallback.
- New `update_manager/remote-web` Vite TypeScript project that builds the
  `saturn-remote-next.js` IIFE bundle consumed by `saturn-remote-next.html` via
  `<script src="/remote-assets/remote-next.js">` (mapped by
  `rust-server/src/remote_tls.rs`). The bundle exposes its API as
  `globalThis.SaturnRemoteNext` for the inline page script.
- Saturn Remote beta checkpoint for `/remote-next`: Phase 42 split
  control/media WebSockets, Phase 44 Opus wideband TX, the conservative ESSB
  CFC baseline, the ESSB-lite TX EQ curve, and operator-controlled Noise Gate
  are now the documented beta path. Field validation showed clear Chrome
  Android TX audio with `accepted=opus_wb`, `codecDecodeFaults=0`,
  `codecPcmFallback=0`, and `txMicDrops=0`.
- Fresh Nginx installs now redirect `/remote-next` and `/saturn/remote-next`
  to the queryless TLS entry point. The Rust TLS listener owns the current
  stable feature defaults, avoiding a second stale redirect configuration.
- New shared `update_manager/scripts/saturn-go-web-assets.sh` web asset
  manifest sourced by both `install_saturn_go_nginx.sh` and
  `update-saturn-go.sh`, so installs and Saturn Go self-updates deploy the
  same set of HTML/JS assets to `/var/lib/saturn-web/`.
- Optional Tailscale package install mode for `install_saturn_go_nginx.sh`,
  enabled with `SATURN_INSTALL_TAILSCALE=1`. The installer only installs the
  package when requested and prints the operator follow-up steps; joining the
  tailnet and configuring Tailscale Serve remain explicit operator actions.
- Hidden `/p23test` lab and `p23-app-manager.sh` now document and drive the
  converged `p2app` service path directly instead of presenting `P2_app` and
  `P3_app` as separate active deploy targets. Legacy `p3` action arguments are
  still accepted as compatibility aliases.
- G2/app-status wording now distinguishes runtime app identity from deployed
  binary family, which makes the transition state readable when an older `p3`
  deploy family is still running the converged `p2` identity.
- P2/P3 runtime telemetry (`/p23_perf`) now includes a structured `fpga`
  section with live product, firmware name/version, bit-file date code, clock
  state, fallback-image state, and die-temperature data exported directly from
  the running app instead of relying only on startup-banner text.
- FPGA flashing now uses a dedicated root-owned helper (`saturn-flash-fpga.sh`)
  under `/usr/local/lib/saturn-go/scripts`, with the web-facing
  `flash_fpga.sh` reduced to a narrow `sudo -n` wrapper. This fixes the FPGA
  Flash page when the service user does not have general passwordless sudo.
- Saturn Go XDMA tools now include a `Stage Running Kernel` maintenance action
  in `saturngo.html`, backed by a narrow privileged helper that runs
  `saturn-fix-xdma.sh --stage-kernel "$(uname -r)"` without restarting
  `p2app.service`.
- `saturngo.html` now includes a compact XDMA status card that surfaces the
  parsed doctor stage/advisory state without requiring users to read the full
  terminal dump.
- Rust server test coverage for the extracted handler/helper modules (`auth`,
  `middleware`, `pages`, `update`, `util`, `monitor`, `clone`, `image`,
  `repair`), including endpoint-level checks for CSRF enforcement, update
  policy/status handlers, the shared `begin_update_activity` conflict gate, Pi
  image request validation, and repair-pack/verify-system-config response
  shape.
- Graceful shutdown: server now handles SIGINT/SIGTERM via `axum::serve().with_graceful_shutdown()`, allowing in-flight requests to complete before exit.
- Runtime repo-tree discovery/switching API: `GET /list_repo_roots`, `POST /set_repo_root`, with persisted active root (`repo_root.txt`).
- Backup UI controls for selecting and applying the active repo root.
- Rust backend health endpoint: `GET /healthz`.
- CSRF request guard for mutating (`POST`) API routes using `X-Saturn-CSRF`, plus same-host Origin/Referer checks when present.
- Appliance update API: `GET/POST /update_policy`, `POST /update_start`, `GET /update_status`, `POST /update_rollback`.
- Transactional repo update flow with staged git worktree switch and rollback on health-check failure.
- Pre-update snapshot archives with retention policy in `/var/lib/saturn-state/snapshots`.
- Backup UI "Appliance Update" panel for channel policy, start/status, and rollback controls.
- `saturn-go-watchdog.timer` + `saturn-go-watchdog.service` for periodic health checks and self-heal restart.
- Backup / Restore clone UI `Wipe Target` action plus `POST /pi_wipe_target` endpoint for quick pre-clone metadata wipe (best-effort unmount, signature/partition-table cleanup, first/last 16 MiB zeroing).
- Dedicated Saturn Go self-update page (`/saturngo`, `/saturn-go`) with separate repo/ref policy form, live terminal output, and rebuild/redeploy workflow for `saturn-go.service`.
- Saturn Go self-update policy API (`GET/POST /saturngo_policy`) with a separate persisted policy file from the G2 Appliance Update policy.
- Saturn Go deploy status endpoint (`GET /saturngo_deploy_status`) backed by a persisted status JSON file for last-run/deploy visibility.
- New `update-saturn-go.sh` script to update repo/build/redeploy the Rust backend from the web UI via `/run`.
- Hidden P2/P3 App Test Lab page (`/p23test`) for build/deploy/switch testing of `P2_app` and `P3_app` without adding navigation links.
- P2/P3 test-lab status endpoint (`GET /p23_status`) reporting service state, source/deployed binaries, symlink selection, and systemd override state.
- P2/P3 test-lab performance endpoint (`GET /p23_perf`) for lightweight process/network/XDMA/PCIe snapshots used to baseline P3/P2 runtime behavior and investigate lag.
- P2/P3 ADC peak telemetry toggle endpoint (`POST /p23_adc_telemetry`) and hidden `/p23test` panel for enabling/disabling runtime ADC peak export without persistent disk writes.
- New `p23-app-manager.sh` helper script for P2/P3 build/deploy/switch/revert actions via `/run`.
- New `scripts/install-shutdown-waiter-service.sh` migration installer to deploy `saturn-shutdown-waiter.service`, initialize `/etc/default/saturn-shutdown-waiter`, and remove legacy `~/.config/autostart/g2-shutdown.desktop`.
- Hidden P2/P3 Test Lab now includes a `Capture Snapshot` action that packages the current `/p23_perf` payload, derived runtime counters, and baseline sample summary into a copyable/downloadable JSON snapshot for lab review.
- Hidden P2/P3 Test Lab snapshot capture now also carries the effective `p2app.service` runtime-tuning state from `systemctl show -p Environment`, so lab snapshots can record live `SATURN_P3_RT_AUDIO_*` settings alongside host/app metrics.
- G2 Update page now includes a read-only `Show App / Firmware Info` action that prints the active P2/P3 app/version from `/p23_perf` plus the latest `p2app.service` startup banner lines for FPGA firmware, build date, bit-file date code, clocks, and startup die temperature.
- G2 Update now includes `Update Web Manager Too`; when `update-G2.py` detects pulled changes under `update_manager/`, the page can automatically launch `update-saturn-go.sh --skip-git --verbose` as a separate final post-step after the G2 run completes, rebuilding from the already-updated active repo root.
- Dedicated deskHPSDR Update page (`/deskhpsdr`) and `update-deskhpsdr.py` runner for live clone/update/build terminal workflow, including helper-script-driven dependency/build flags and fresh-image clone support.
- Default Custom Scripts startup seeding now also includes `setup-eth-fallback.sh`, exposing the Ethernet APIPA fallback repair helper in the web manager.
- Default Custom Scripts startup seeding now also includes `fix-LED-power-button.sh`, exposing the front-panel LED/power-button repair helper in the web manager.
- Saturn Go page now includes an `XDMA Doctor` action that runs a classified read-only PCIe/XDMA report through the existing privileged helper lane.

### Changed
- Saturn Remote TLS now treats plain `/remote-next` and `/remote-next.html`
  as default operator entry points and redirects them to
  `/remote-next?transport=split&tx_opus=1&tx_cfc=1`. Operator logs and bridge
  runtime messages use feature names instead of development phase numbers.
  Existing `phase40_*`, `phase42_*`, and `phase44_*` bookmarks and browser
  storage remain compatible and are silently canonicalized to stable names.
  Saturn Go self-deploys also migrate persisted Nginx redirects to the
  queryless entry point and validate/reload Nginx transactionally.
- `update-deskhpsdr.py` v1.1 now compacts routine apt/debconf/autoremove
  chatter in verbose web output, while keeping the raw build log intact.
  Its build helper no longer reinstalls the PulseAudio daemon on
  `pipewire-pulse` systems and now applies the Saturn libgpiod v2 patch only
  for older upstream deskHPSDR checkouts that still contain the legacy
  `src/gpio.c` path. Current upstream checkouts, where direct Raspberry Pi
  GPIO support has been removed, build with `SATURN=ON` and skip the obsolete
  patch.
- Saturn Remote TLS listener now fails closed when `SATURN_REMOTE_BASIC_AUTH`
  is unset or malformed: it refuses to bind on port 8443, logs an `ERROR`
  with the remediation, and leaves the Saturn Go admin HTTP listener
  (port 8080, `/saturn/*` via nginx) running so the appliance stays
  manageable. Set `Environment=SATURN_REMOTE_BASIC_AUTH=username:password`
  in `saturn-go.service` (typically via the installer-managed drop-in or
  `systemctl edit saturn-go.service`) to restore `/remote` and
  `/remote-next`. As a development-only escape hatch, set
  `SATURN_REMOTE_DEV_INSECURE=1` to start the listener without auth — not
  for production. Pre-existing deployments running without the env var
  will see `:8443` stop binding after upgrade until they set it.
- `install_saturn_go_nginx.sh` now writes
  `/etc/systemd/system/saturn-go.service.d/10-remote-auth.conf` (mode
  `0600 root:root`) carrying `SATURN_REMOTE_BASIC_AUTH=admin:<password>`
  alongside `/etc/nginx/.htpasswd` whenever the installer captures a
  fresh admin password (interactive prompt, `SATURN_ADMIN_PASSWORD` env,
  or non-TTY random generation). Reruns that reuse an existing
  `/etc/nginx/.htpasswd` preserve any pre-existing drop-in unchanged.
  When the installer has no fresh password and no drop-in exists, it
  warns the operator with the exact `systemctl edit` recipe needed to
  align the TLS path with the LAN nginx password.
- Known gap: `/change_password` (admin password change UI) currently
  updates only `/etc/nginx/.htpasswd`. The Saturn Remote TLS drop-in must
  be updated manually (or the installer rerun with `SATURN_ADMIN_PASSWORD`)
  until `/change_password` is extended to write both targets. The
  Tailscale helper (`saturn-go-tailscale-serve.sh`) refuses to configure
  Serve when this misalignment leaves `/remote-next` returning anything
  other than HTTP 401 to unauthenticated requests.
- `install_saturn_go_nginx.sh` now installs `nodejs` and `npm` as required
  dependencies and runs `npm ci && npm run build` in
  `update_manager/remote-web` before staging assets, so a fresh install
  produces and deploys `saturn-remote-next.js` automatically.
- `saturn-remote-next.html` and `saturn-remote-next.js` are now treated as
  required deploy assets in `saturn-go-web-assets.sh`. Installs and Saturn Go
  self-updates fail loudly if the Vite build does not produce
  `remote-web/dist/saturn-remote-next.js`, instead of silently shipping a
  documented `/remote-next` URL with a missing bundle.
- Saturn Remote TX control now treats PTT as a low-latency control-plane event:
  the browser keys immediately before microphone startup completes, stops
  enqueueing mic frames before sending PTT-off, uses smaller TX mic frames, and
  the bridge no longer blocks the main loop waiting for WDSP TX slew-down on
  release.
- Saturn Remote entry points now send operators to the current beta
  `/remote-next?phase42_split=1&phase44_tx_opus=1&phase44_tx_cfc=1`
  checkpoint with cache marker `bridgeprefill240-cfcessb3`, instead of the
  older Opus-only gate-off test URL.
- `update-G2.py` and `update-saturn-go.sh` now verify that configured policy
  repos/refs are publicly reachable over HTTPS before pulling, use the
  validated policy URL directly instead of rewriting the local git remote, and
  surface a clear "public repo required" error when a private or mistyped
  GitHub repo is saved for unattended/anonymous update flows.
- Appliance Update now follows the same anonymous-safe policy URL behavior:
  the Rust backend probes the configured public repo/ref with `git ls-remote`,
  fetches the selected ref directly from the policy URL into `FETCH_HEAD`, and
  leaves the local checkout's `origin` remote untouched.
- `update.html` and `saturngo.html` now warn that saved GitHub repos must be
  public for unattended or anonymous appliance/self-update runs.
- `g2-version-info.sh` now prefers live `/p23_perf` FPGA/runtime metadata for
  firmware version, product/version, bit-file date code, fallback state, and
  die temperature, using the `p2app.service` startup-banner journal scrape only
  as a fallback when runtime telemetry is unavailable.
- `g2-version-info.sh` now labels the selected deployed slot separately from
  the app's live runtime protocol identity so the current transition state
  (`p3` slot running as `p2`) reads clearly instead of looking contradictory.
- `g2-version-info.sh` now emits an explicit warning when `/p23_perf` fetches
  fail or return no data instead of silently falling back to the non-telemetry
  path.
- The FPGA flash helper now resolves `--latest` with a safe per-file mtime scan
  instead of relying on `ls -t "$FPGA_DIR"/*.bin`.
- `g2-version-info.sh` now reports the deployment slot separately from the
  live app identity, avoids printing two competing `Runtime` blocks, and scopes
  the FPGA startup-banner scrape to the current `p2app.service` instance
  instead of grepping the full unit journal.
- `saturn-xdma-doctor.sh` now emits an explicit advisory when XDMA is working
  only because the module is already loaded but the module is not installed on
  disk for the running kernel, so the doctor output distinguishes "runtime OK"
  from "reboot/recovery safe".
- The privileged XDMA doctor/staging helpers now re-exec through
  `systemd-run --pipe` when launched from `saturn-go.service`, so Saturn Go
  can inspect or stage `/lib/modules/.../xdma.ko` without inheriting the
  service's kernel-module protection sandbox.
- `scripts/fix-xdma.sh` now falls back to `/usr/src/linux-headers-<kernel>`
  and repairs missing `/lib/modules/<kernel>/{build,source}` links before
  failing header detection, which fixes false "headers missing" failures on
  kernels whose header package is already installed.
- `update_manager/rust-server/src/main.rs` is no longer carrying shadow copies
  of the server subsystems. The extracted modules (`state`, `util`, `update`,
  `auth`, `middleware`, `pages`, `monitor`, `clone`, `image`, `repair`) are
  now wired into the live binary, and `main.rs` has been reduced to router and
  remaining shared runtime logic instead of a parallel monolith plus orphaned
  code paths.
- Backup/restore test tooling now carries forward the active clone/image
  request behavior: clone start supports `verify_compare`, stderr
  classification is normalized in the clone job log, and `pi_wipe_target`
  lives beside the rest of the clone-device workflow instead of remaining
  inline in `main.rs`.
- Hidden P2/P3 Test Lab `/p23test` now honors the shared light/dark browser
  theme preference and includes an in-page theme toggle instead of forcing a
  dark-only palette.
- Saturn Go installer/self-update now also deploys the root-owned `saturn-xdma-doctor.sh` helper into `/usr/local/lib/saturn-go/scripts` and extends the narrow sudoers policy so the non-root web service can invoke it via `/opt/saturn-go/scripts/xdma-doctor.sh`.
- Appliance update policy now includes `healthcheck_retries` and
  `healthcheck_initial_delay_secs`, allowing staged repo switches and rollback
  probes to tolerate slower local service startup before declaring health-check
  failure.
- Startup repo-root resolution now canonicalizes both the default root and any
  saved `repo_root.txt` path before Saturn repo validation, closing symlink
  bypasses in the early startup path.
- Browser-managed custom script metadata is now bounded at load and write time:
  script count, flag count, and flag length are capped, and oversized
  `custom_scripts.json` files are rejected before deserialization.
- Full restore tar preflight now parses verbose tar listings so restore can
  validate archive paths and symlink targets, measure uncompressed size, reject
  extreme expansion ratios, and check `/tmp` free space before extraction.
- Update G2 now runs the shutdown waiter installer plus
  `fix-LED-power-button.sh` as part of the maintenance flow.
- `setup-eth-fallback.sh` remains available in Custom Scripts, but no longer
  auto-runs during Update G2.
- Installer now deploys `install-shutdown-waiter-service.sh` and
  `shutdown-waiter.sh` into the runtime script set, installs matching
  root-owned helper copies under `/usr/local/lib/saturn-go/scripts`, and
  writes a narrow sudoers policy so the web UI can execute the three G2 repair
  helpers with `sudo -n`.
- Shutdown waiter install/provision defaults now use `SATURN_SHUTDOWN_WAITER_ENABLED_DEFAULT=auto`, so fresh images arm the post-boot power-button watcher unless hardware auto-detect says not to.
- Saturn Go installer/self-update script sync now includes the repo-root `scripts/setup-eth-fallback.sh` helper in the managed `/opt/saturn-go/scripts` set.
- Saturn Go installer/self-update script sync now also includes the repo-root `scripts/fix-LED-power-button.sh` helper in the managed `/opt/saturn-go/scripts` set.
- deskHPSDR update/build flow now applies a Saturn-managed `deskhpsdr` libgpiod v2 compatibility patch before build and probes with `GPIO=ON` instead of forcing `GPIO=OFF`, preserving GPIO support on Trixie during update-manager driven builds.
- P2/P3 app speaker and DUC underrun telemetry now counts underflow episodes on edge transitions instead of incrementing on every FIFO-monitor poll while the same starvation condition is still active. This makes `/p23_perf` cumulative underrun history comparable across runs and prevents one recovery period from inflating the totals.
- P2/P3 app high-priority telemetry now preserves the live ADC peak-hold values when exporting `/dev/shm/saturn_p23_perf_stats.json`, so the lab page ADC gauges match runtime packet content instead of being zeroed before snapshot.
- P2/P3 app Makefiles now emit and include header dependency files, preventing stale object reuse after telemetry enum/header changes; this fixed shifted `/p23_perf` counters such as bogus wideband send errors and corrupted DUC/speaker totals after incremental rebuilds.
- `update-saturn-go.sh` now deploys the same web/script asset set as provisioning: release binary, HTML pages, `config.json`, `themes.json`, and packaged runtime scripts, while preserving browser-managed extra scripts in `/opt/saturn-go/scripts`.
- Hidden P2/P3 Test Lab `/p23test` now uses a workload-first dashboard layout instead of a single dense text block, making host pressure and app behavior easier to read together.
- Hidden P2/P3 Test Lab status and dashboard panes now surface the effective service runtime/audio-RT configuration, and `Copy JSON` falls back to a legacy textarea copy path when the browser Clipboard API is unavailable.
- `/p23_perf` now includes workload tags plus optional app-exported telemetry (`/dev/shm/saturn_p23_perf_stats.json`) so the lab can correlate host metrics with DDC/wideband/mic/DUC/speaker activity.
- ADC peak telemetry documentation now matches runtime behavior: disabling the
  feature removes the enable flag and stops updates, but the last `/dev/shm`
  snapshot is retained until overwritten or removed.
- CSRF middleware now rejects POST requests that are missing both `Origin` and `Referer` headers, closing a bypass when neither header was sent.
- `/get_system_data`: `proc_regex` query parameter now compiled with a 64 KB size limit via `RegexBuilder` to prevent regex-based CPU exhaustion.
- `/exit` endpoint now logs the remote IP at `warn` level before initiating shutdown.
- Pi image download cleanup delay increased from 30 seconds to 10 minutes, preventing file deletion while large downloads are still in progress.
- Completed Pi image and clone job maps are now pruned to a maximum of 20 entries, preventing unbounded memory growth over long uptimes.
- Default custom script constants replaced with `include_str!()` referencing `scripts/cleanup-saturn-logs.sh` and `scripts/cleanup-saturn-backups.sh`, eliminating source duplication.
- Updated `README.md` to document the current Rust/Axum backend, deployment layout, and compatibility naming (`saturn-go` service/binary).
- Documented version panel behavior and the non-interactive privilege model used by `update-G2.py` and `/change_password`.
- Replaced installer implementation with a Rust-only deployment flow (no embedded Go source generation/build).
- Installer now configures NGINX as a path-prefix reverse proxy for `/saturn/*` plus a dedicated SSE route for `/saturn/run`.
- Installer now enforces non-default admin bootstrap credentials (prompt/env/random generation), instead of shipping `admin/admin`.
- Re-aligned uninstall script to remove the exact artifacts created by current install flow (service, NGINX site, SSE map, optional auth/runtime purge).
- Uninstall now defaults to keeping runtime directories/custom state; use `--purge` for full cleanup.
- Installer script sync now preserves browser-managed custom scripts and only updates packaged scripts when source files are newer.
- Request body handling gained explicit global and restore upload limits;
  REM-0501 later replaced the overly broad global allowance with route-specific limits.
- Main, backup, and monitor web UIs now attach `X-Saturn-CSRF: 1` to all mutating API calls.
- `run` SSE path now streams with lower latency: line-buffered subprocess invocation (`stdbuf` when available), `\r` + `\n` boundary handling, and no-cache/anti-buffer response headers.
- NGINX `/saturn/run` now disables request buffering and adds explicit no-cache header to reduce end-to-end stream latency.
- Monitor refresh interval reduced from 3s to 1s for more appliance-like real-time visibility.
- Installer now writes update-policy/snapshot/staging env configuration into `saturn-go.service`.
- Installer now uses dedicated writable state path (`/var/lib/saturn-state`) for repo-root and appliance update state files.
- Installer now applies additional systemd hardening (`RestrictSUIDSGID`, `ProtectKernel*`, `ProtectControlGroups`, syscall/address-family restrictions).
- Installer now omits `NoNewPrivileges` in `saturn-go.service` so allowed `sudo -n` maintenance paths remain functional.
- Repo-root switching and restore now require Saturn-style git checkout paths (`.git` + `update_manager`), preventing destructive restore targets.
- Appliance update now prunes older staged worktrees under `/var/lib/saturn-state/repo-staging` to limit disk growth.
- Uninstaller now removes watchdog units and watchdog script to stay aligned with installer artifacts.
- `/run` now blocks Python script execution when the resolved script path is inside the active Saturn repo tree; only installed script copies are allowed.
- Python scripts launched by `/run` now set `PYTHONDONTWRITEBYTECODE=1` and `PYTHONPYCACHEPREFIX=/var/cache/saturn-python`.
- Clone target detection now includes USB-attached block devices that report `removable=0` (common for USB SD card readers), while still excluding internal/virtual devices.
- Saturn Go self-update page now uses explicit run-option checkboxes (`verbose`, `dry-run`, `skip-git`, `skip-build`, `skip-deploy`) and shows a polling "Last Deploy Status" panel.
- `/run` now injects Saturn Go self-update policy env vars and a deploy-status-file path when launching `update-saturn-go.sh`, and treats Saturn Go self-update as a shared update activity (conflict-guarded like G2/appliance update).
- Installer now copies `saturngo.html` and hidden `p23test.html`, writes Saturn Go policy/deploy-status service env vars, and auto-bootstrap builds with a rustup-managed stable toolchain (removing legacy apt `cargo`/`rustc` when present).
- Installer now warns before `apt-get update` when `/tmp` is too full (helps diagnose misleading APT signature-split errors caused by tmpfs exhaustion).
- Hidden P2/P3 Test Lab now supports explicit startup profiles (`panel`, `panel-debug`, `headless`) and front-panel mode overrides (`SATURN_FRONT_PANEL_MODE`) for switch/deploy actions, plus an emergency web revert button.
- `p23_status` now reports parsed override metadata (`panel_mode`, `saturn_meta`) from the generated systemd drop-in for easier troubleshooting.
- `p23_status` now also reports optional ADC peak telemetry state and the latest exported snapshot from `/dev/shm/saturn_p23_adc_peak_telemetry.json`.
- Hidden P2/P3 Test Lab now includes a Phase 1 performance panel with resettable baselines (process CPU/RSS/scheduler wait plus `eth0` throughput and XDMA interrupt-rate/PCIe link telemetry).
- Hidden P2/P3 Test Lab performance panel now highlights threshold alerts for scheduler wait / CPU spikes and XDMA interrupt spikes or drops relative to baseline.
- `P2_app` / `P3_app` ADC peak telemetry export now writes only a single latest-snapshot JSON file in `/dev/shm` while enabled, capped to roughly once per second, instead of continuously writing persistent logs.
- Hidden P2/P3 Test Lab status/performance panes no longer restore stale cached content across reloads, browser polling is serialized, and the performance baseline resets automatically when `p2app.service` restarts into a new P2/P3 process identity.
- `/p23_perf` CPU total-tick parsing now excludes Linux guest accounting fields so process CPU percentages are not slightly under-reported when guest time is present.
- `/p23_perf` now includes process `rchar`/`wchar` counters, and the hidden P2/P3 Test Lab derives an XDMA efficiency proxy (`IRQ/MiB` of process char I/O) so high raw XDMA interrupt rates can be compared against actual app traffic.
- `update-G2.sh` and `update_manager/scripts/update-G2.py` now run the shutdown-waiter migration installer as part of update flow.
- Provisioning (`provision/cloud-init/provision-saturn.sh`) now installs and runs the shutdown-waiter installer by default (`SATURN_INSTALL_SHUTDOWN_WAITER=1`) and adds `gpiod` to required packages.
- `scripts/copy-autostart.sh` no longer installs `g2-shutdown.desktop`; shutdown waiting is now service-managed.
- `autostart-files/g2-shutdown.desktop` is now marked deprecated/hidden to prevent new desktop-session activation.
- FPGA flash page (`templates/fpga.html`) now uses `/run_log` as the primary source of truth, labels write/verify phases separately, keeps polling until buffered output is fully drained, removes stale session-state replay, and limits rendered output to a smaller visible tail to reduce browser lockups during long flash/verify runs.
- FPGA flash live log polling now serializes in the browser and batches DOM updates to avoid overlapping fetch/render cycles during high-volume `load-FPGA` output.

### Fixed
- deskHPSDR builds now apply a startup guard that prevents the Saturn XDMA
  Connect path from dereferencing `active_receiver` before receiver creation
  finishes, and the generated Desktop shortcut is now a directly executable
  application launcher instead of an unreliable filesystem `Type=Link`.
- Appliance update and rollback health checks no longer fail on the first
  transient timeout by default; the local health probe can now wait and retry
  according to normalized policy values.
- Full restore now rejects unsafe symlink targets and archives that would
  over-expand or exceed current `/tmp` free space before extraction begins.
- `update-G2.py`: verbose mode now preserves captured command output used by status sections (fixes `Size: ?` and `Commit: ?` cases).
- `update-G2.py`: library install now checks which APT packages are actually missing before attempting privileged installs.
- `update-G2.py`: privileged steps now use adaptive sudo behavior (`sudo` with TTY, `sudo -n` without TTY) with clearer failure messages.
- `/change_password`: backend now retries with `sudo -n htpasswd` and returns actionable permission errors when sudo rules are missing.
- `/get_versions`: missing script `version` metadata now returns `unknown` instead of a misleading hard-coded version.
- Added explicit `version` metadata for `update-G2.py` (`2.14`) and `update-pihpsdr.py` (`1.10`) in `scripts/config.json`.
- `/change_password`: switched to `htpasswd -i` with stdin input so passwords are not exposed in process arguments.
- `restore-backup.sh`: fixed `--list --json` runtime error and malformed JSON output.
- `restore-backup.sh`: added `--backup-name` support used by web UI restore flow.
- `/run_log` now tracks absolute line offsets for rolling script logs so long FPGA flash runs continue streaming correctly after the in-memory buffer begins truncating older lines.
- `monitor.html`: replaced process-row `innerHTML` injection with DOM `textContent` rendering for safer output handling.
- `scripts/config.json`: corrected restore script `directory` path metadata.
- Installer health check now fails loudly (with `systemctl`/`journalctl` diagnostics) if backend `/healthz` does not come up.
- Installer now always restarts `saturn-go.service` after writing the unit, preventing stale in-memory env (e.g., old bind port) on reinstall.
- Password minimum reduced to 5 characters across installer prompt, backend `/change_password`, and web UI validation.
- Light-theme terminal output now keeps compile/build lines readable (ANSI white mapped for light backgrounds), including the dedicated `pihpsdr.html` terminal view.
- `update-G2.py`: added an automatic CAT signature compatibility patch for `sw_projects/P2_app/g2panel_libgpiodv2.c` after pulls, fixing recent `MakeProductVersionCAT` build breaks.
- `update-G2.py`: `install_udev_rules` now skips with warning (instead of hard-failing the full update) in non-interactive mode when passwordless sudo is unavailable.
- G2 terminal runner now enforces a configured Appliance Update repo URL; if not configured, `/run` returns a clear error instead of silently using defaults.
- G2 terminal runner now passes Appliance Update policy repo/remote/ref into `update-G2.py`, and `update-G2.py` now applies that policy by setting the git remote URL before pulling.
- `update-G2.py` and `update-pihpsdr.py` now refuse execution when run from inside the Saturn repo tree, preventing accidental repo-local Python runs.
- `update-pihpsdr.py`: prevented startup crash on non-UTF-8 (`latin-1`) stdout/stderr/log streams by adding per-stream Unicode fallback output and explicit UTF-8 log file writes.
- `update-pihpsdr.py`: verbose dependency installation now runs apt/debconf in noninteractive mode and compacts routine apt autoremove/debconf noise instead of flooding the piHPSDR terminal log.
- `update-pihpsdr.py` v1.12: added automatic WDSP 2.00 Linux compatibility for the renamed PureSignal `doPSCorrChange` worker and opaque event handles, plus a dependency preflight that reuses installed libraries without launching the privileged upstream installer on every web update.
- `update-deskhpsdr.py` v1.2: hardened clean-image builds by suppressing upstream's redundant interactive `sudo apt-get` calls after Saturn's privileged prerequisite check, compacting package/download progress, and documenting resumable no-clean recovery.
- Saturn Go watchdog now requires three consecutive 10-second health-check failures before restarting the backend, preventing a single missed check during CPU-heavy radio builds from terminating an otherwise healthy update job.
- `update-saturn-go.sh`: fixed `--dry-run` staging-helper generation error when the staged directory is intentionally not created.
- `update-saturn-go.sh`: fixed detached root-helper status-file error handling/JSON quoting so `/saturngo_deploy_status` always returns valid JSON after deploy completion.
- `p23-app-manager.sh`: dry-run deploy/switch no longer writes a temp override file (avoids failing when `/tmp` is full).
- `index.html`: custom scripts/version/flags/password UI now reports clearer errors when an API returns HTML (login page / backend error page) instead of JSON.
- `scripts/shutdown-waiter.sh`: added config-gated modes (`auto|true|false`), pull-up pin reads, high-before-arm guard, and consecutive-low confirmation to reduce false shutdown triggers on mixed hardware variants.
- `/p23_perf` now falls back to `eth0` byte counters as a documented char-I/O proxy source (`process.io.source = "eth0_netdev_proxy"`) when `/proc/<pid>/io` is unreadable for the running `p2app.service` process, keeping the hidden P23 `IRQ/MiB` metric usable under service-user permission constraints.
- `scripts/shutdown-waiter.sh` now tolerates both libgpiod CLI styles when reading the shutdown GPIO, trying the newer `gpioget -c/--chip ... --numeric` form first and falling back to the older positional-chip syntax so Bookworm/Trixie and older images both work.

## [2026-02-13]
### Added
- Full repo backup download (`/backup_full`) and restore (`/restore_full`) with validation and RESTORE confirmation.
- Dedicated **Backup / Restore** page (`backup.html`) linked from the main UI.
- Pi image creation workflow with progress, validation (size + SHA256), and download cleanup.
- Output directory selection for Pi image creation (default `/mnt/usb`).
- Pi image cancel support and live log panel.
- Clone SD card to removable device workflow with auto-detected targets, progress, and cancel.
- Repair Pack download and system config verification tools.
- New script `clone_pi_to_device.sh`.

### Changed
- Disabled default request body limits to support large uploads.
- Removed **Create Pi Image** from the main script list (moved to backup page).
- Added `SATURN_REPO_ROOT` env var for configurable repo root.

### Fixed
- Restores now accept `dry_run=1` and similar boolean query values.
