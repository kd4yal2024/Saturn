# Operations Runbook

## Scope

This runbook covers day-2 operations for the Rust-based Saturn Update Manager deployment.

## Build and Install

### Build Backend Locally

```bash
cd /home/pi/github/Saturn/update_manager/rust-server
cargo check
cargo build --release
```

### Full Install (recommended)

```bash
cd /home/pi/github/Saturn
sudo ./install.sh
```

Installer actions include:

- installs dependencies (`nginx`, `apache2-utils`, build tools, Python tools, etc.)
- removes legacy distro `cargo`/`rustc` packages (if present)
- bootstraps/updates a current stable Rust toolchain via `rustup` for the build user, then validates Cargo can read the repo lockfile
- builds and deploys Rust binary to `/opt/saturn-go/bin/saturn-go`
- provisions exact pinned WDSP 2.00 and piHPSDR Linux-port source commits in an
  installer-owned sparse cache, then builds and installs
  `/opt/saturn-go/bin/saturn-bridge`
- copies web assets to `/var/lib/saturn-web`
- copies scripts to `/opt/saturn-go/scripts`
- grants service-user ownership of `/opt/saturn-go/scripts` so browser-managed custom script edits can persist
- installs root-owned privileged helper copies to `/usr/local/lib/saturn-go/scripts`
- rewrites `/etc/sudoers.d/saturn-go-maintenance` for those privileged helper paths
- writes NGINX config for `/saturn/*` and SSE route `/saturn/run`
- writes `saturn-go.service`, `saturn-bridge.service`, watchdog service, and
  watchdog timer
- waits for target-aware backend readiness at `/readyz`, including the exact
  full Git commit embedded in the newly built binary

The normal installer does not require cloud-init to clone DSP sources, build
piHPSDR, or create the bridge service. Set `SATURN_INSTALL_BRIDGE=0` only for an
intentional backend-only installation. The default is fail-closed: Remote UI
assets are not considered a complete install if their matching bridge cannot
be built.

## Update Existing Deployment

After pulling changes that modify host configuration, trusted helpers, sudoers,
nginx, driver policy, or systemd units, apply the canonical installer contract:

```bash
cd /home/pi/github/Saturn
sudo ./install.sh
```

The installer resumes matching completed phases and reapplies the full contract
when the repository commit or host schema changes. Use `--force` only for an
intentional complete reprovision.

### Versioned release activation status

Milestone 2 can build and install complete inactive releases under
`/opt/saturn/releases/<full-commit>`. REM-0203 also installs a root-owned
activation broker. REM-0204 adds automatic restoration of the prior pointer,
systemd drop-ins, service activity, and exact-commit readiness when activation
fails. Production activation is still intentionally disabled until this
transaction passes a separately approved live appliance rollback test.

An operator may validate an installed release without changing the active
pointer or restarting anything:

```bash
sudo /usr/local/lib/saturn-go/scripts/saturn-release-activate-root.sh \
  --validate <full-commit>
```

Do not set `ACTIVATION_ENABLED=1` in
`/etc/default/saturn-release-activate` during normal installation. The
installer keeps it disabled. The current legacy deployment remains active
until a live activation and deliberate rollback test are explicitly approved.

Build an inactive release from a branch or tag without trusting the mutable ref
after selection:

```bash
cd /home/pi/github/Saturn
update_manager/scripts/saturn-release-build.sh \
  --source-remote https://github.com/kd4yal2024/Saturn.git \
  --source-ref remediation/reliability-hardening
```

The builder records the requested ref, its canonical full ref, source remote,
and resolved 40-character commit in the schema-v3 manifest. It fails if the
remote ref moves between resolution and fetch, then builds and tests only the
detached resolved commit. Use `--resolve-only` to inspect the selection without
building. Neither operation activates a release or restarts services.

The durable transaction record is
`/var/lib/saturn-state/deployments/current.json`. A successful recovery records
`status: rolled_back`; an incomplete recovery records
`status: rollback_failed` and prevents another activation until an operator
resolves it. The rollback snapshot is stored beside that record under
`rollback-current/`. Activation never prunes installed immutable releases.

REM-0205 adds a persistent-state contract to every new application release.
The current contract is state schema 1, an initial metadata-only marker over
the existing state schema 0; the managed settings formats themselves are
unchanged. Before any required migration, the activator backs up the marker
and declared settings files to:

```text
/var/lib/saturn-state/deployments/state-backups/<timestamp>-<full-commit>/
```

Each backup has a root-owned manifest with sizes, modes, owners, and SHA-256
digests. Restore validation rejects missing, corrupt, oversized, non-regular,
or redirected payloads before replacing live state. Migration and backup
details are also recorded in the deployment transaction.

The activation helper blocks a target that cannot read the current state or
does not declare support for its immediately preceding schema. If a future
migration is explicitly documented as one-way, it still requires a local root
operator to use:

```bash
sudo /usr/local/lib/saturn-go/scripts/saturn-release-activate-root.sh \
  --approve-one-way-migration <full-commit>
```

That flag is intentionally unavailable through Saturn Go sudoers. Do not use
it merely to bypass an incompatibility error. Production activation remains
disabled until the separately approved live rollback test.

Saturn Go self-update also builds the bridge in build-only mode, stages the
binary and service unit with the UI bundle, and deploys them together. Set
`SATURN_SATURNGO_BUILD_BRIDGE=0` only when intentionally retaining an older
bridge.

Validate the complete payload without changing running services:

```bash
SATURN_ACTIVE_REPO_ROOT=/home/pi/github/Saturn \
  bash update_manager/scripts/update-saturn-go.sh --skip-git --stage-only --verbose
```

This builds Saturn Go, Saturn Bridge against the pinned WDSP 2.00 sources, and
the Remote UI bundle, then checks the generated deployment helper. During a
real update, a failed service startup or bridge TCI listener check restores the
previous service binaries.

### Graceful Saturn Go shutdown

Check shutdown admission without changing service state:

```bash
curl -fsS http://127.0.0.1:8080/shutdown_status | jq
```

`accepting_jobs: true` is the normal state. `POST /exit`, SIGINT, and SIGTERM
first change it to `false`; new maintenance jobs are then rejected. Active
transactional or unclassified jobs finish, while only explicitly cancel-safe
cleanup/report scripts receive process-group SIGTERM and, after the configured
grace period, SIGKILL. Their maintenance record ends in `cancelled` rather than
being inferred as an unexplained interruption after restart.

Do not use `POST /exit` as a health probe: it intentionally stops Saturn Go and
`Restart=on-failure` does not restart a clean exit. Announce and approve any
live service stop/restart before testing it. A normal approved
`systemctl stop|restart saturn-go.service` follows the same controller path;
the generated unit uses `KillMode=mixed` and `TimeoutStopSec=15min` so the main
process can apply the declared job policy before systemd escalates.

### WDSP 2.00 / PureSignal Post-Install Check

Verify the deployed bridge and UI are from the same installation:

```bash
systemctl --no-pager status saturn-bridge.service
nm -a /opt/saturn-go/bin/saturn-bridge \
  | grep -E 'SetRXAWBFMdmph|SetTXAPHROTAutoMode|SetPSControl|pscc'
sha256sum -c /var/lib/saturn-web/saturn-remote-next.js.sha256
```

For the first PureSignal test, close Thetis and other Protocol 2 controllers,
use a 50-ohm dummy load, start at 5 W or less, enable automatic feedback
attenuation, and use the 700/1900 Hz two-tone generator. Expected healthy
telemetry is feedback near 140-165, `correcting=1`, stable attenuation, and zero
feedback gaps. Stop immediately on ADC overload, a feedback fault, or an
unexpected power reading.

The bridge installer keeps Cargo/native source caches under
`update_manager/saturn-bridge/target-local`. Saturn Go self-update saves the
previous deployed bridge as `/opt/saturn-go/bin/saturn-bridge.previous` before
replacement. To roll that binary back:

```bash
sudo systemctl stop saturn-bridge.service
sudo install -m 0755 /opt/saturn-go/bin/saturn-bridge.previous \
  /opt/saturn-go/bin/saturn-bridge
sudo systemctl start saturn-bridge.service
```

If an older install attempt failed with a Cargo lockfile parse error (for example
`lock file version '4'` on Bookworm using distro `cargo`), rerun the installer.
Current installer versions self-bootstrap a newer Rust toolchain via `rustup`.

Remote entry behavior:

- `http://<host>/remote` should redirect to `https://<host>:8443/remote-next` (with the default feature query).
- `http://<host>/saturn/remote` should redirect the same way.
- `https://<host>:8443/remote` and the TLS root `/` redirect to `/remote-next`; the legacy inline page was retired on 2026-07-14.
- `https://<host>:8443/remote-next?transport=split&tx_opus=1&tx_cfc=1` is the current Saturn Remote default operator URL: split control/media sockets plus the guarded Opus TX path and conservative ESSB CFC baseline. It serves `saturn-remote-next.html` + Vite bundle `saturn-remote-next.js` via `/remote-assets/remote-next.js`. Older `phase42_*` and `phase44_*` flags remain accepted as compatibility aliases.
- `https://<host>:8443/remote-next` without a query redirects to the default feature query. It is the only remote UI; basic-auth credentials, `remote_settings.json`, and `remote_profiles.json` state are unchanged from the legacy page.
- Saturn Remote accepts at most four authenticated logical clients. Each split
  control/media pair sharing a `session` id counts once; the bridge separately
  caps physical TCI sockets at eight. A fifth logical client is rejected with
  HTTP 429 and a five-second retry hint. This is an appliance capacity limit,
  not an authentication failure.
- Exactly one connected control client remains the operator/TX owner. Other
  clients are viewers until the operator disconnects and ownership is promoted.
  Disconnect, TX watchdog, codec fault, and media-lane loss still force RX.
- Remote control state is coalesced in fixed 256-entry inbound and per-client
  outbound queues. Outbound control is additionally capped at 256 KiB,
  display is latest-frame depth one, microphone ingress keeps eight newest
  frames, and audio retention is limited to 250 ms. TX release and safety
  commands use the priority lane and are processed before control or media.
- The current Opus TX processing profile leaves Noise Gate available as an operator control but off by default, keeps TX EQ on with the restrained Voodoo curve `2:+2,3:+4,4:+3,5:+1,6:-1,8:+2`, and enables the conservative ESSB CFC baseline only when `tx_cfc=1` is present. Noise Gate can be explicitly started on for testing with `tx_noise_gate=1` and a guarded threshold such as `tx_noise_gate_db=-50`; once loaded, the operator Noise Gate toggle is allowed to persist instead of being repeatedly reset by the TX audio restore path.
- Field validation on 2026-06-11 confirmed `bridgeprefill240-gateoff1` transmitted clear Opus wideband TX audio from Chrome Android with `accepted=opus_wb`, `txNoiseGateEnabled=0`, `txMicDrops=0`, and `txUplinkHwm=854`.
- Field validation on 2026-06-11 also found the wider TX filter `50-4150` sounded good with `bridgeprefill240-gateoff2`; copy-log evidence showed `accepted=opus_wb`, `txMicDrops=0`, and `txUplinkHwm=637`.
- `https://<host>:8443/remote-next?transport=split&tx_opus=1&tx_cfc=1` enables the opt-in ESSB CFC baseline. The baseline keeps CFC conservative: precomp `1.0 dB`, CFC bands `50:+0.5, 100:+1.0, 200:+1.5, 500:+2.0, 1k:+1.5, 1.5k:+1.0, 2.5k:+0.5, 3k:+0.5`, and no CFC lift above 3 kHz. This is intended to add bottom-end warmth and warm mids without the older high-gain CFC profile that caused choppy TX audio. Optional `tx_cfc_precomp_db=<0..4>` can tune precomp for tests.
- TX Setup includes the built-in **Voodoo 3.8k** audio profile. Applying it sets a true `50-3850 Hz` passband, `-6 dB` microphone gain, the restrained EQ/CFC curves above, and Noise Gate off at a retained `-50 dB` threshold. It intentionally does not change drive, tuning, PTT/MOX, antenna, or any other RF state. Existing saved profiles remain untouched until the operator applies Voodoo 3.8k and saves the desired radio profile.
- Field validation on 2026-06-12 confirmed `bridgeprefill240-cfcessb2` held clear Opus wideband TX audio with CFC enabled: `accepted=opus_wb`, `codecDecodeFaults=0`, `codecPcmFallback=0`, `txMicDrops=15`, and `txUplinkHwm=4192` during a longer Chrome Android TX. The small browser drop count came from the Opus uplink hard cap and was not reported audible.
- Additional field validation confirmed the same CFC profile with cleaner short-run counters: `accepted=opus_wb`, `codecDecodeFaults=0`, `codecPcmFallback=0`, `txMicDrops=0`, `txUplinkHwm=596`, `iqStreaming=1`, and `audioStreaming=1`; operator report was "TX Audio is clear, working really well."
- Noise Gate validation confirmed the operator toggle and threshold controls persist under the TX audio restore path: `txNoiseGateEnabled=1`, `txNoiseGateThresholdDb=-35.0`, `accepted=opus_wb`, `codecDecodeFaults=0`, `codecPcmFallback=0`, `txMicDrops=0`, `txUplinkHwm=3318`, and the operator reported "Noise Gate working fine."
- Shared remote settings persist in `/var/lib/saturn-state/remote_settings.json`.
- Named remote Setup profiles persist in `/var/lib/saturn-state/remote_profiles.json`.
- The remote `Setup` menu supports profile save/load/delete, startup profile selection, and panadapter/waterfall display presets.
- The remote `Setup -> DSP` menu supports server-backed `NR` and `ANF` controls, including taps, delay, gain, and leakage.
- The remote meter panel now applies local smoothing for S-meter, TX power, and SWR so the analog gauges track like an operator console instead of stepping on raw samples.
- The TX power meter supports `Peak` and `Avg` display modes.
- Browser disconnect and `pagehide` now send an explicit TX-off command before the TCI websocket closes, and the bridge also forces TX/two-tone off when the client detaches.
- Two-tone test settings now persist in `remote_settings.json`, including `freq1`, `freq2`, `level`, `invert LSB-family`, and `tone 2 delay`.
- USB/LSB mode changes should move the transparent RX passband box to the correct side of center in both the panadapter and waterfall.

Remote capacity diagnostics:

```bash
curl -ksS -u admin https://127.0.0.1:8443/remote_metrics | jq
curl -fsS http://127.0.0.1:8080/bridge_diag | jq '.bridge.journal.latest_diag.fields'
```

The first response reports authenticated client/connection counts, rejection
counters, and high-water marks. The bridge diagnostic reports physical
connection ceilings, command and outbound queue depths, coalesced/replaced and
dropped traffic, safety/control latency, and byte/TCP high-water marks. The
same bridge scheduling data is emitted to connected operators in the
`remote_backpressure` TCI message.

Remote Setup profile notes:

- `remote_settings.json` holds the active working state, including the current active profile name.
- `remote_settings.json` also carries the current DSP and TX test-control preferences used by the remote.
- `remote_profiles.json` holds the saved profile catalog plus the optional startup profile selection.
- A startup profile should be applied before opening a live phone session when you want a known panadapter, waterfall, and radio-control baseline.
- If the Setup menu opens underneath the panadapter after a deploy, confirm the latest `saturn-remote-next.html` + `saturn-remote-next.js` were synced into `/var/lib/saturn-web/`.
- If USB/LSB signals or the transparent passband box appear on the wrong side of center after a deploy, confirm the latest `saturn-remote-next.html` + `saturn-remote-next.js` were synced into `/var/lib/saturn-web/`.
- If `/remote-next` returns 404 on the bundle (`/remote-assets/remote-next.js`), confirm lockfile-only `npm ci && npm run build` succeeded in `update_manager/remote-web` and that `saturn-remote-next.js` plus `saturn-remote-next.js.sha256` are present in `/var/lib/saturn-web/`. The installer and `update-saturn-go.sh` now treat a missing bundle or checksum mismatch as a hard failure; this check covers manual or pre-promotion deploys.
- If TX appears stuck after a browser crash or tab close, confirm both `saturn-bridge.service` and `saturn-go.service` are on the latest deployed build with the explicit TX-release path.

## Secure Remote Access with Tailscale VPN

Tailscale is the recommended way to reach Saturn Remote from outside the LAN. It is **optional** — nothing in Saturn requires it, and the existing LAN entry points keep working unchanged. The deployment described below provides operator-friendly remote access (real Let's Encrypt cert, MagicDNS hostname, no port-forwarding) while preserving every Saturn security control: HTTP basic auth, the visible RF TX gate, and loopback-only internal listeners. Direct Internet port forwarding is unsupported.

### Why Tailscale Serve, not Funnel

Use `tailscale serve` to expose Saturn Remote **only to your tailnet**:

```bash
sudo tailscale serve --bg --https=443 https+insecure://127.0.0.1:8443
```

Operator URL after Serve is configured:

```
https://<saturn-host>.<tailnet>.ts.net/remote-next
```

Do **not** use `tailscale funnel` for normal operation. Funnel exposes the same listener to the public internet via Tailscale's edge, which puts a radio control surface in front of arbitrary internet traffic. The basic-auth gate is the only line of defense in that configuration, and a single credential leak becomes a worldwide problem instead of a tailnet-scoped one.

### Why `https+insecure://127.0.0.1:8443` (and not `http://127.0.0.1:8080`)

Saturn Go listens on two addresses:

- `127.0.0.1:8080` — internal HTTP listener for `/saturn/*` admin routes proxied via nginx. No basic-auth gate, no `/remote*` routes, no `Cross-Origin-Opener-Policy` / `Cross-Origin-Embedder-Policy` headers.
- `0.0.0.0:8443` — Saturn Remote TLS listener (`rust-server/src/remote_tls.rs`). Enforces the basic-auth check (`check_remote_basic_auth`), serves `/remote`, `/remote-next`, `/remote-assets/*`, and `/tci`, and emits the COOP/COEP headers that `/remote-next` requires for `SharedArrayBuffer` and `AudioWorklet` to function in the browser.

Tailscale Serve must front the `:8443` listener, not `:8080`. The `https+insecure://` scheme tells Tailscale to skip cert validation against the loopback origin (Saturn's self-signed cert) — the public-facing cert that browsers see is the real Let's Encrypt cert Tailscale provisions for the tailnet hostname. This is the documented Tailscale pattern for fronting a self-signed origin.

A future contributor "simplifying" the mapping to `:8080` would silently strip basic auth and the cross-origin isolation headers and ship a broken `/remote-next` page. The mapping is load-bearing.

### Tailscale URL rule: no port

The operator-facing URL is always:

```
https://<saturn-host>.<tailnet>.ts.net/remote-next
```

with **no explicit port**. `tailscale serve --https=443` binds 443 on the tailnet hostname; browsers default to 443 for `https://`, so the port is implicit and omitting it is correct.

The existing nginx config at `/etc/nginx/sites-available/saturn` returns `302 https://$host:8443/remote` for plain-HTTP `/remote` hits. That redirect is **LAN-only behavior**. Over Tailscale it would bounce operators off the Serve port (443) onto port 8443, which is not exposed by the Serve mapping. Document the no-port URL explicitly in any operator-facing setup notes; do not type `:8443` into a tailnet URL bar.

### Hostname hygiene

The tailnet hostname inherits the system hostname when the node first joins. A stock Raspberry Pi OS install gives every Pi the same `raspberrypi` hostname, which collides with every other Pi on the operator's tailnet — first-come wins, the rest get suffixed (`raspberrypi-1`, `raspberrypi-2`, ...).

Before running `tailscale up` for the first time, set a meaningful hostname:

```bash
sudo tailscale set --hostname=saturn-g2
```

(`saturn-g2`, `saturn-shack`, `kd4yal-saturn` — anything operator-meaningful and unique within the tailnet.)

If the node is already joined under a generic name, the same command updates the tailnet hostname; the node's URL changes immediately, and stale `<old>.<tailnet>.ts.net` URLs stop resolving.

### Authentication layers

Tailscale does **not** replace Saturn Remote's basic auth — it stacks on top.

- **Tailscale**: gates *who can reach* the listener at all. Only authenticated tailnet members (and their explicitly shared devices) can route traffic to the Pi.
- **Basic auth in Saturn Remote**: gates *who can use* the listener once reached. The nginx LAN admin path uses `/etc/nginx/.htpasswd`; the Saturn Remote TLS listener uses the `SATURN_REMOTE_BASIC_AUTH=username:password` service environment consumed by `rust-server/src/remote_tls.rs`. Both credential paths are kept aligned by `saturn-admin-password.sh` (used by the UI change-password flow and console `reset`); never edit one side by hand. Survives Tailscale account compromise, shared device misuse, and accidental ACL widening.
- **`SATURN_REMOTE_TX_RF_ENABLED`**: gates *whether RF TX is permitted at all*. The bridge installer enables it by default for Saturn Remote operation. Set it to `0` in the bridge environment to disable RF TX without changing authentication, Tailscale exposure, or the browser PTT controls.

These three controls are independent. Do not collapse them.

#### Live audit finding (2026-05-02)

When the Tailscale helper was first dry-run on the development Pi, it correctly refused to configure Serve because `SATURN_REMOTE_BASIC_AUTH` was **not set** in `saturn-go.service`'s environment, and `https://127.0.0.1:8443/remote-next` was returning HTTP 200 to unauthenticated requests. The TLS remote auth gate was silently fail-open — the listener was reachable on the LAN with no credential check.

This is a fail-open failure mode of `rust-server/src/remote_tls.rs::check_remote_basic_auth`: when `configured_basic_auth_header()` returns `None` (env var absent or malformed), the function returns `Ok(())` and every `/remote*` route serves unauthenticated. Saturn warns at startup but does not refuse to start.

Remediation applied on that Pi:

```bash
sudo install -d -m 0755 /etc/systemd/system/saturn-go.service.d
sudo tee /etc/systemd/system/saturn-go.service.d/10-remote-auth.conf >/dev/null <<'EOF'
[Service]
Environment=SATURN_REMOTE_BASIC_AUTH=admin:<choose-five-characters>
EOF
sudo chmod 0600 /etc/systemd/system/saturn-go.service.d/10-remote-auth.conf
sudo systemctl daemon-reload
sudo systemctl restart saturn-go.service
curl -k -sS -o /dev/null -w 'HTTP %{http_code}\n' https://127.0.0.1:8443/remote-next
# expected: HTTP 401
curl -k -sS -o /dev/null -w 'HTTP %{http_code}\n' -u admin:<password> https://127.0.0.1:8443/remote-next
# expected: HTTP 200
```

Operators inheriting an existing deployment should run the unauthenticated `curl` check above before assuming the basic-auth gate is active. Code-level fail-closed hardening (refuse to start the TLS listener when the env var is absent) is tracked separately and discussed in the next subsection.

The manual drop-in recipe above is the historical remediation from that audit. On current installs, set or re-align the credential with the password helper instead — it updates `/etc/nginx/.htpasswd` and the TLS drop-in together so the LAN admin path (`/saturn/*`) and the TLS remote path (`/remote*`) cannot drift:

```bash
printf '%s\n' 'abc12' | sudo /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh set --restart now
sudo /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh status   # expect sync_state=in_sync
```

#### Fail-closed remote TLS listener (current behavior)

The Saturn Remote TLS listener now fails closed when basic auth is not configured:

- The listener on port 8443 refuses to bind if `SATURN_REMOTE_BASIC_AUTH` is unset or malformed (no `username:password` separator, empty username, or empty password).
- The Saturn Go admin HTTP listener on port 8080 (`/saturn/*` via nginx) keeps starting normally — Saturn Go remains manageable from the LAN admin path even when the remote TLS gate refuses.
- Startup emits an `ERROR` log line naming the missing env var and the remediation command, plus a follow-up `ERROR` confirming the admin listener is unaffected.

To temporarily start the TLS listener without basic auth (development only — DO NOT use in production), set:

```bash
sudo systemctl edit saturn-go.service
# Add under [Service]:
#   Environment=SATURN_REMOTE_DEV_INSECURE=1
sudo systemctl restart saturn-go.service
```

Saturn logs a warning and binds the listener with no auth gate. The override exists as an escape hatch for dev/lab environments and irregularly-upgraded appliances; long-term we expect operators to set `SATURN_REMOTE_BASIC_AUTH` and never touch the override.

The installer writes `/etc/systemd/system/saturn-go.service.d/10-remote-auth.conf` (mode 0600 root:root) carrying `SATURN_REMOTE_BASIC_AUTH=admin:<password>` whenever a fresh password is captured during install (interactive prompt, `SATURN_ADMIN_PASSWORD` env, or non-TTY generation). Reruns preserve an existing credential only when both nginx and Saturn Remote backends are present. An incomplete legacy state is repaired by selecting or generating a new five-character password; the installer never leaves the two backends knowingly out of sync.

The `/change_password` admin endpoint calls the privileged `saturn-admin-password.sh set` helper, which updates `/etc/nginx/.htpasswd` **and** the TLS auth drop-in together, then schedules a deferred `saturn-go` restart (~2s) so the TLS listener picks up the new credential. The two backends cannot drift through normal use. To audit or recover:

```bash
sudo /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh status   # sync_state=in_sync expected
sudo /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh reset    # console recovery: prints a fresh five-character password
```

`reset` is deliberately console-only (physical access is the trust anchor); there is no remote reset path. The Tailscale helper (`saturn-go-tailscale-serve.sh`) additionally refuses to configure Serve when the unauthenticated `curl` check returns anything other than 401.

### Bridge bind audit (known wide-bind on TCI)

Verify what is actually listening before claiming anything is private:

```bash
sudo ss -ltnp | grep -E "saturn|8080|8443|50001"
```

Expected on a current deploy:

- `127.0.0.1:8080` — saturn-go internal HTTP. Loopback. Correct.
- `0.0.0.0:8443` — saturn-go TLS. Wide. Correct (basic-auth gated).
- `0.0.0.0:50001` — **saturn-bridge TCI listener. Wide. Currently unauthenticated.**

The bridge TCI listener is wide-bound by deployment-time configuration: `/etc/systemd/system/saturn-bridge.service` sets `Environment=SATURN_BRIDGE_TCI_HOST=0.0.0.0`. The repo's `saturn-bridge.service.example` does not set that variable, so the source default of `127.0.0.1` (`saturn-bridge/src/config.rs:36-37`) only applies when the deployed unit doesn't override it.

This is a pre-existing LAN exposure, not a Tailscale regression. On the LAN it lets external TCI clients (Thetis, log4om, etc.) drive the radio over the wire without authentication. **Tailscale amplifies this** because every device on the operator's tailnet — phones, work laptops, devices shared by other tailnet users — inherits the same reach.

Until a hardening pass tightens the bridge bind, operators using Tailscale should:

- Treat tailnet membership as equivalent to LAN access for TCI purposes.
- Use Tailscale ACLs to restrict the saturn node to specific user/device groups, not the default "everyone in the tailnet" reach.
- Avoid sharing the Saturn node with guest devices via Tailscale's device-share feature.

A planned future hardening pass will scope the bridge TCI listener (e.g. bind to `tailscale0` only, or require an auth handshake on connect) so this stops being an honor-system control. Tracked separately from this Tailscale rollout.

### Tailscale ACL recommendation

Default Tailscale ACL is `accept: ["*:*"]` — every node can reach every other node on every port. For a radio appliance this is too broad. Recommended baseline (set in the Tailscale admin console):

```jsonc
{
  "tagOwners": {
    "tag:saturn-radio": ["autogroup:admin"]
  },
  "acls": [
    {
      "action": "accept",
      "src":    ["autogroup:admin"],
      "dst":    ["tag:saturn-radio:443"]
    }
  ]
}
```

Tag the saturn node with `tag:saturn-radio` (`sudo tailscale up --advertise-tags=tag:saturn-radio --reset`), and only allow admin-tagged users to reach port 443 on it. Phones and laptops still have to authenticate; guest devices and shared nodes do not get implicit reach to the radio.

Tag-based ACLs are documented as an operator improvement, not a default — Tailscale's free tier supports them and they are the right model long-term, but the helper script does not enforce them.

### Cert renewal

Tailscale Serve auto-renews the Let's Encrypt cert for the tailnet hostname while the node is online and the tailnet is reachable. Renewal happens roughly 30 days before expiry.

A node that is offline for **more than ~90 days** can come back to an expired cert. Reconnecting to the tailnet and waiting for the next renewal cycle (or running `sudo tailscale cert <hostname>` to force) clears it. For always-on Saturn Pis this is not a real concern; flag it for operators who run portable or seasonal installations.

### Reboot validation

After `tailscale up` and `tailscale serve` have been configured, reboot the Pi and verify:

```bash
# Daemon comes back automatically.
systemctl is-enabled tailscaled.service
systemctl is-active tailscaled.service

# Tailnet connection re-established.
tailscale status

# Serve mapping is persistent (Tailscale stores it in tailscaled state).
tailscale serve status

# /remote-next loads from the tailnet URL after cold boot.
curl -fsI "https://<saturn-host>.<tailnet>.ts.net/remote-next" \
  -u admin:<password> >/dev/null && echo "remote-next reachable"
```

All four should pass without operator intervention. If `tailscaled` is not enabled, run `sudo systemctl enable --now tailscaled` so the daemon auto-starts on boot. Serve mapping is persistent in Tailscale's local state and does not need to be re-issued; the daemon coming back is sufficient.

### Client validation matrix

Before declaring a Tailscale rollout production-ready, validate the following clients against `https://<saturn-host>.<tailnet>.ts.net/remote-next`:

- Windows: Chrome, Edge
- macOS: Safari, Chrome
- iPadOS: Safari
- iPhone: Safari
- Android: Chrome (if available)

For each client, validate:

- Page loads (`/remote-next` HTML + bundle).
- WSS websocket connects (browser DevTools → Network → WS).
- RX audio plays without underruns (requires `SharedArrayBuffer`, which requires the COOP/COEP headers from the TLS listener — confirms the `https+insecure://127.0.0.1:8443` mapping is wired correctly).
- Panadapter and waterfall render.
- Microphone permission prompt fires on PTT arm.
- PTT arm → key → unkey cycle does not leave the bridge in TX state (`journalctl -u saturn-bridge.service` should show the explicit TX-release packets after release).
- Reconnect after sleep / browser backgrounding / Tailscale client sleep restores the WSS without a full page reload.

Any failure on Safari (iPad/iPhone/macOS) is the highest-priority signal — Safari's stricter cross-origin isolation and audio capture rules are where most production surprises live.

### What does not change

- LAN access via `https://<lan-ip>:8443/remote-next` and `https://<lan-ip>:8443/remote` continues to work exactly as before.
- The nginx admin proxy at `http://<lan-ip>/saturn/` is unaffected.
- Basic-auth credentials stay aligned between the LAN nginx path (`/etc/nginx/.htpasswd`) and the Tailscale/Saturn Remote TLS path (`SATURN_REMOTE_BASIC_AUTH`): password changes go through `saturn-admin-password.sh`, which updates both together. Verify any time with `sudo /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh status`.
- Saturn Go self-update, FPGA flash, backup/restore, and all other admin workflows continue to use the LAN nginx path, not the Tailscale URL.

Tailscale is purely an additional access path for `/remote` and `/remote-next`. It does not replace, gate, or alter any other Saturn workflow.

## GitHub Commit and Push

From the Saturn repo root:

```bash
cd /home/pi/github/Saturn
git status --short
git add <paths>
git commit -m "Describe the change"
git push origin HEAD
```

If you need a new branch:

```bash
cd /home/pi/github/Saturn
git checkout -b <branch-name>
git add <paths>
git commit -m "Describe the change"
git push -u origin <branch-name>
```

Operational notes:

- Review `git status --short` first so you do not accidentally commit unrelated local work.
- Use `git push --force-with-lease` only when you intentionally rewrote already-pushed history.
- If a commit message contains an unwanted trailer, fix it with `git commit --amend` before pushing.

## Uninstall

```bash
cd /home/pi/github/Saturn
sudo bash update_manager/uninstall_saturn_go_nginx.sh [--purge] [--no-purge] [--keep-auth] [--remove-packages] [--dry-run] [--yes]
```

Default behavior keeps runtime directories (including custom scripts and state):

- `/opt/saturn-go`
- `/var/lib/saturn-web`
- `/var/lib/saturn-state`

Use `--purge` for a full cleanup.

## Service Operations

### Status and Logs

```bash
sudo systemctl status saturn-go.service
sudo journalctl -u saturn-go.service -n 200 --no-pager
sudo systemctl status saturn-go-watchdog.timer
sudo systemctl status saturn-fftw-wisdom.timer
sudo /usr/local/lib/saturn-go/scripts/saturn-fftw-wisdom.sh --status
```

### Restart

```bash
sudo systemctl restart saturn-go.service
sudo systemctl restart saturn-go-watchdog.timer
```

### FFTW Wisdom Maintenance

Saturn Bridge imports the machine-local cache at
`/var/cache/saturn-bridge/wdspWisdom01`. The installer creates it with Saturn's
Rust FFTW planner and enables a low-priority, persistent weekly timer. The
timer checks a hardware/software fingerprint and does not rebuild a fresh
cache.

```bash
# Inspect freshness without changing anything
sudo /usr/local/lib/saturn-go/scripts/saturn-fftw-wisdom.sh --status

# Run the normal fingerprint-aware check now
sudo systemctl start saturn-fftw-wisdom.service

# Force regeneration (may take significant time on a Raspberry Pi)
sudo /usr/local/lib/saturn-go/scripts/saturn-fftw-wisdom.sh --rebuild

# Review planner output
sudo journalctl -u saturn-fftw-wisdom.service -n 200 --no-pager
```

Generation uses `FFTW_PATIENT`, runs with low CPU and I/O priority, and never
opens the radio backend. A generated cache becomes active the next time
`saturn-bridge.service` starts. If the cache is missing or invalid, the bridge
logs the condition and safely performs normal runtime FFTW planning.

### Front-Panel Power Button

The shutdown installer selects one owner for the physical power button:

- A live native gpio-keys input named `pwr_button` owns shutdown through
  `KEY_POWER` and systemd-logind. The GPIO polling waiter is disabled.
- Hardware without that native input retains `saturn-shutdown-waiter.service`
  as the guarded GPIO26 fallback.
- BCM15 belongs to the red/white front-panel LED and is not the button input.

On Raspberry Pi OS desktop images, `/etc/xdg/autostart/pwrkey.desktop` starts a
low-level `handle-power-key` inhibitor. For a native Saturn power button, the
installer writes a Saturn-managed `~/.config/autostart/pwrkey.desktop` override
for the configured operator. Reboot or log out/in once so the already-running
desktop inhibitor exits; logind then performs the clean poweroff.

Inspect the selected policy without changing state:

```bash
sudo /usr/local/sbin/saturn-shutdown-waiter.sh --diagnose
cat /proc/bus/input/devices | grep -A7 -B2 -E 'shutdown_button|pwr_button'
sudo systemd-inhibit --list --no-pager
```

If diagnostics report both a native button and an active
`dtoverlay=gpio-shutdown,gpio_pin=26`, the same switch may be registered twice.
The installer warns but deliberately leaves `/boot/firmware/config.txt`
unchanged. Back it up, confirm both input devices target GPIO26, comment only
the verified duplicate overlay, and reboot. Do not change the button to GPIO20
without hardware evidence.

Waiter messages are available in the normal unit journal:

```bash
sudo journalctl -u saturn-shutdown-waiter.service -b --no-pager
```

### NGINX Validation

```bash
sudo nginx -t
sudo systemctl reload nginx
```

### API Quick Checks

Through NGINX (authenticated session in browser) or locally against backend:

```bash
curl -fsS http://127.0.0.1:8080/livez
curl -fsS http://127.0.0.1:8080/readyz | jq .
curl -fsS http://127.0.0.1:8080/update_status
curl -fsS http://127.0.0.1:8080/list_repo_roots
curl -fsS "http://127.0.0.1:8080/run_log?script=update-G2.py&from=0&limit=20"
```

## Key Workflows

### Navigation Layout

- `/saturn/` opens the Saturn Go Overview dashboard; G2 Update remains at
  `/saturn/update`.
- `/saturn/monitor` opens real-time system monitoring.
- `/saturn/telemetry` opens Radio Telemetry & Diagnostics.
- `/saturn/remote-next` redirects to the Saturn Remote TLS operator console.
- `/saturn/saturngo` opens dedicated Saturn Go self-update page.
- `/saturn/pihpsdr` opens dedicated piHPSDR update page.
- `/saturn/deskhpsdr` opens dedicated deskHPSDR update page.
- `/saturn/fpga` opens dedicated FPGA flash page.
- `/saturn/backup` opens Backup / Restore.
- `/saturn/tailscale` opens Tailscale VPN configuration and status.
- `/saturn/custom` (and `/saturn/index`) opens Custom Scripts page.
- Navigation is grouped as primary operator views (Overview, Monitor, Radio
  Telemetry, Saturn Remote), Maintenance, Applications, and System.

### Repo Root Management

- Use `backup.html` repo root controls, or call:

```bash
curl -sS -X POST http://127.0.0.1:8080/set_repo_root \
  -H 'Content-Type: application/json' \
  -H 'X-Saturn-CSRF: 1' \
  -d '{"repo_root":"/home/pi/github/Saturn"}'
```

Validation requires `.git` and `update_manager/` in the target path.

### Backup and Restore

Read `STATE_INVENTORY.md` and `BACKUP_FORMATS.md` before relying on a backup.
The Backup page now separates:

- **Settings Backup**: portable-with-review Saturn settings, registered
  operator scripts, and direct piHPSDR/deskHPSDR property files. The archive
  is private operator data and explicitly excludes credentials and device
  identity. Import is transactional and skips host policy by default.
- **Source Backup**: the complete active Saturn repository, including local
  changes and anything else below that root. It is not an appliance backup.
  The old `/backup_full` URL remains an alias for compatibility.
- **Installed Release Backup**: one selected manifest-bearing immutable
  release under `/opt/saturn/releases/<full-commit>`, without mutable state.

Likewise, `saturn-backup-*`, `pihpsdr-backup-*`, and `deskhpsdr-backup-*`
directories are source-tree rollback copies. The piHPSDR repository copy
normally carries root-level `*.props`; the deskHPSDR source copy does not carry
`~/.config/deskhpsdr/*.props`. REM-0205 state backups contain only the direct
managed Saturn state files needed to undo a release-state migration.

- Download settings with `GET /backup_settings`.
- Download source with `GET /backup_source`.
- List/download immutable releases with `GET /backup_releases` and
  `GET /backup_release?commit=<full-commit>`.
- Validate/import settings with `POST /restore_settings?dry_run=1`, then apply
  only after `confirm=RESTORE`. Leave host-policy import off unless deliberately
  moving this appliance's repository/update configuration.
- Validate source with `POST /restore_source?dry_run=1`, then apply only after
  `confirm=RESTORE`. `/restore_full` remains a compatibility alias.
- For script-created directory backups, use Backup page "Script Backups" controls:
  - Saturn backups from `update-G2.py`: `GET /g2_backups`, `POST /g2_restore`
  - piHPSDR backups from `update-pihpsdr.py`: `GET /pihpsdr_backups`, `POST /pihpsdr_restore`

Important:

- source restore creates a new flushed repository generation, switches the
  repository pointer atomically, and retains the previous checkout
- settings restore keeps durable rollback copies and startup recovery rolls
  back an incomplete transaction before the service accepts requests
- immutable-release archive import/activation remains a separate local
  release-manager operation
- none of these source-repository restores reconstructs an appliance
- upload size is limited by `SATURN_RESTORE_MAX_UPLOAD_BYTES`; nginx and the
  backend stream these routes without buffering the complete request body
- uploads and extraction preserve `SATURN_READY_MIN_FREE_BYTES` (512 MiB by
  default) on `$SATURN_STATE_DIR/restore-tmp`; failure to determine free
  capacity fails the restore closed
- non-dry-run full restore acquires the shared update lock; concurrent update actions return `409 Conflict`

### Manual Whole-Disk Imaging

Saturn Go intentionally does not create card images, clone cards, enumerate
clone targets, or wipe removable devices. Perform these destructive operations
from a local terminal where the operator can inspect the physical device.

Before every operation, identify the source and target by model, serial number,
size, transport, and mount points:

```bash
lsblk -o NAME,PATH,SIZE,MODEL,SERIAL,TRAN,RM,TYPE,MOUNTPOINTS
```

To create an image of the running `/dev/mmcblk0` on mounted external storage:

```bash
cd /home/pi/github/Saturn
sudo ./update_manager/scripts/make_pi_image.sh --out-dir /mnt/usb --compress
```

To clone to a removable device, replace `/dev/sdX` only after verifying it with
`lsblk`. The target is completely overwritten:

```bash
cd /home/pi/github/Saturn
sudo ./update_manager/scripts/clone_pi_to_device.sh --target /dev/sdX
```

Do not run either command remotely over an unreliable VPN or SSH connection.
Prefer powering down and using a separate workstation/card reader when making
a distribution master image.

### Appliance Update

1. Open G2 Update page (`/saturn/update`).
2. Enter GitHub repo URL and branch/ref.
3. Configure health-check URL and timeout.
4. Save settings (optional; Start also persists current values).
5. Start update.
6. Monitor `update_status` job until complete.
7. If needed, run rollback.

Current UI behavior:

- UI persists policy using `channel=custom` and `custom_ref=<branch/ref>`.
- Appliance policy panel stores repo/ref/health settings consumed by both transactional Appliance Update and `Run Update G2`.
- Appliance Update requires the configured GitHub repo/ref to be publicly
  reachable over HTTPS for anonymous appliance users. It fetches directly from
  the saved policy URL and does not rewrite the local checkout's `origin`.
- `Run Update G2` requires valid Appliance repo URL before run can start.
- `Run Update G2` auto-saves current Appliance settings before spawning script.
- Terminal output is resumable after tab/page changes using buffered `/run_log` polling.
- Update G2 also runs the installed shutdown waiter and LED/power-button repair
  helpers as part of the maintenance flow.
- Ethernet fallback remains available as a manual Custom Script, not an
  automatic Update G2 step.

G2 Update coordination notes:

- Update G2 terminal and Appliance Update now live on the same page.
- If Appliance Update already moved Git to target commit, run Update G2 with `--skip-git`.
- G2 runs and appliance update/rollback are mutually exclusive; overlapping requests return `409 Conflict`.

Update behavior:

- updates Git remote to expected GitHub URL from policy
- fetches target ref
- snapshots active repo (if enabled)
- stages update in `repo-staging` worktree
- switches active repo root only after staging
- health-check gates completion; failed checks auto-revert root

### Update G2 (Dedicated Terminal)

- Run `update-G2.py` from the G2 Update page to keep terminal output and Appliance Update state together.
- Use `Show App / Firmware Info` on the same page for a read-only status pull without starting an update run.
- `Update Web Manager Too` is enabled by default and only takes effect when the current `Run Update G2` actually pulls changes under `update_manager/`.
- The chained post-step runs `update-saturn-go.sh --skip-git --verbose`, so it rebuilds/redeploys from the already-updated active repo root and does not need a second Saturn Go git pull.
- Repo URL in Appliance section must be valid before G2 run is enabled.
- Backend injects active repo-root environment for `/run`:
  - `SATURN_REPO_ROOT`
  - `SATURN_DIR`
  - `SATURN_ACTIVE_REPO_ROOT`
- `/run` rejects Python script execution when the resolved script path is inside active `SATURN_REPO_ROOT`.
- Python runs from `/run` set `PYTHONDONTWRITEBYTECODE=1` and `PYTHONPYCACHEPREFIX=/var/cache/saturn-python`.
- This allows `update-G2.py` to target the active Saturn checkout without hardcoded path dependence.
- Output can be resumed from backend run logs (`/run_log`) after navigating away and back.
- The info action runs `g2-version-info.sh` via `/run` and prints:
  - deployment slot and live app identity/version from `/p23_perf`
  - current deployed app binary path
  - the current `p2app.service` startup banner lines for FPGA product/firmware/build/date-code info when that start emitted them
  - otherwise, the most recent retained banner block if one still exists in the journal
  - the startup die-temperature line only when it was captured in those banner lines
- When `Run Update G2` pulls new commits, `update-G2.py` now emits a marker if any changed path is under `update_manager/`.
- If that marker is present and `Update Web Manager Too` is checked, the page automatically launches `update-saturn-go.sh --skip-git --verbose` after the G2 run finishes.
- The follow-up Saturn Go self-update remains a separate final step, so the active G2 run is not interrupted mid-update; expect the page to disconnect briefly when `saturn-go.service` restarts near the end of that follow-up step.
- Installed systems now rely on `/etc/sudoers.d/saturn-go-maintenance` so the
  service user can run the root-owned copies under
  `/usr/local/lib/saturn-go/scripts` with `sudo -n` during web execution.

### Saturn Go Self-Update (Dedicated Terminal + Redeploy)

- Open `/saturn/saturngo`.
- The page stores a separate Saturn Go repo/ref policy (`GET/POST /saturngo_policy`) so it does not overwrite the G2 Appliance Update policy.
- The page runs `update-saturn-go.sh` via `/run` with live terminal output and buffered resume (`/run_log`).
- The page provides explicit run options that map to script flags:
  - `Verbose output` -> `--verbose`
  - `Dry run` -> `--dry-run`
  - `Skip Git update` -> `--skip-git`
  - `Skip build` -> `--skip-build`
  - `Skip deploy` -> `--skip-deploy`
- `Last Deploy Status` panel polls `GET /saturngo_deploy_status` (status JSON written by the script and detached root helper).

Typical test sequence:

1. `--verbose --dry-run --skip-git` (preview all steps)
2. `--verbose --skip-git --skip-build` (fast deploy path using existing release binary)
3. `--verbose --skip-git` (full local rebuild + redeploy)
4. `--verbose` (full git pull + rebuild + redeploy from Saturn Go policy)

Operational notes:

- The script builds from the active repo root selected on Backup / Restore page (`SATURN_ACTIVE_REPO_ROOT`).
- Before compiling, the script verifies/activates a protected build swapfile
  (`/home/pi/saturn-build.swap`, 2 GiB by default) through
  `saturn-go-build-preflight.sh`.
- Saturn Go Rust builds run with the guarded defaults
  `CARGO_BUILD_JOBS=1`, `cargo build --release -j1`, `nice -n 15`, and
  `ionice -c3`; override with `SATURN_SATURNGO_BUILD_JOBS`,
  `SATURN_SATURNGO_BUILD_NICE`, `SATURN_SATURNGO_BUILD_IONICE_CLASS`,
  `SATURN_SATURNGO_BUILD_SWAP_FILE`, and
  `SATURN_SATURNGO_BUILD_SWAP_MIB` only when intentionally testing.
- The same preflight and one-job default apply when
  `install-saturn-bridge.sh` is run directly. This closes the standalone
  bridge-build path that could otherwise let Cargo choose parallel jobs on a
  1 GiB Pi.
- The preflight prints RAM and swap capacity before compiling and refuses to
  create the 2 GiB swapfile unless at least 512 MiB of filesystem space will
  remain. A failure here is intentional: free disk space before retrying the
  build instead of risking an out-of-memory kill or a full root filesystem.
- `ripgrep`, `shellcheck`, `jq`, `rustfmt`, and `clippy` are optional
  maintainer/CI tools, not Saturn runtime dependencies. Install them on a
  development appliance with:

  ```bash
  sudo apt-get install -y ripgrep shellcheck jq
  /home/pi/.cargo/bin/rustup component add rustfmt clippy
  ```
- Validated on a CM4 Saturn G2 on 2026-06-25 with
  `update-saturn-go.sh --skip-git --skip-deploy --verbose`; the guarded build
  completed successfully in 7m 14s with the 2 GiB build swap active and no
  service restart.
- The deploy payload includes the release binary, web assets listed by `scripts/saturn-go-web-assets.sh`, `config.json`, `themes.json`, and packaged scripts from `update_manager/scripts`.
- Browser-managed extra scripts in `/opt/saturn-go/scripts` are left in place; the self-update only refreshes the repo-managed files.
- Final stop/copy/start of `saturn-go.service` is dispatched via detached `systemd-run` helper (`saturn-go-self-deploy-<timestamp>`).
- The web terminal may disconnect when `saturn-go.service` restarts; reload after ~10-20 seconds.
- Some successful lines may still be prefixed `ERR:` in the terminal because `cargo` and `systemd-run` emit informational output on stderr.

### Radio Telemetry & Diagnostics

- Open Radio Telemetry from the primary navigation or browse to
  `/saturn/telemetry` (`/saturn/p23test` remains a legacy alias).
- This page combines live radio/performance telemetry with advanced controls
  for testing the converged `p2app` build/deploy/restart path and override behavior.
- It runs `p23-app-manager.sh` via `/run` and resumes terminal output using `/run_log`.

Capabilities:

- `Status` (script-based status summary)
- `Build p2app`
- `Build + Deploy p2app`
- `Restart With Current Override`
- `Restore Unit Default` (removes Saturn override and restores unit `ExecStart`)
- Separate status panel backed by `GET /p23_status`
- Separate workload/performance dashboard backed by `GET /p23_perf`
- `Capture Snapshot` button for exporting a point-in-time JSON bundle of the live `/p23_perf` sample, derived metrics, current baseline summary, and effective `p2app.service` runtime tuning state seen by the lab
- A separate `Saturn Performance Lab` tab for a warm-up plus fixed measurement window, persistent named runs, operator observations, and a server-side baseline/candidate `ACCEPT`, `REVIEW`, `REJECT`, or `INCOMPATIBLE` verdict; the live service dashboard remains on `Telemetry & Diagnostics`

Implementation details:

- Deployed binary is staged as `/opt/saturn-go/p23-apps/p2app`
- Active launch path is `/opt/saturn-go/p23-apps/current` symlink
- `p2app.service` is redirected via systemd drop-in:
  - `/etc/systemd/system/p2app.service.d/10-saturn-p23-switch.conf`
- Revert action removes that drop-in and reloads systemd
- Restart/deploy overrides can include:
  - startup profile (`panel`, `panel-debug`, `headless`) -> mapped service args
  - `Environment=SATURN_FRONT_PANEL_MODE=...` (`auto`, `g2`, `g2v2`, `prefer-g2`, `prefer-g2v2`, `off`)
- `GET /p23_status` parses the generated override comment metadata (`# saturn-p23 mode=... panel=...`) for display
- `GET /p23_status` also reports the effective `p2app.service` runtime environment subset from `systemctl show -p Environment`, including optional `SATURN_P3_RT_AUDIO_*` settings for P3 audio-thread tuning
- `GET /p23_perf` overlays host metrics with workload tags and live app telemetry exported from `/dev/shm/saturn_p23_perf_stats.json`
- The dashboard baseline resets automatically when the active workload identity changes (PID, binary-family/mode, or routing shape)
- A Performance Lab run aborts if the service PID, telemetry ownership, active-radio state, or workload identity changes. Use the same band, sample rate, receiver count, routing, and client display shape for both runs.
- Full methodology and gate definitions are in `PERFORMANCE_LAB.md`.
- Snapshot `Copy JSON` falls back to a legacy in-page copy path when the browser Clipboard API is unavailable
- The dashboard is organized around:
  - workload identity and app shape
  - host pressure (CPU, scheduler wait, memory, eth0, XDMA)
  - app packet/DMA throughput for DDC, wideband, mic, DUC, and speaker paths
  - app-side error/fifo/overflow deltas
- Speaker and DUC underrun counters now record underrun episodes on false-to-true transitions rather than incrementing on every FIFO-monitor poll while the same underflow condition is still active. Treat `fifo_speaker_under_events` and `fifo_duc_under_events` as cumulative starvation episodes, then use the per-interval delta cards to see whether the problem is still actively occurring.

Safety/usage notes:

- Use `Dry run` first for deploy/restart/revert actions
- Non-dry-run deploy/restart/revert actions require browser confirmation
- Web mode requires `sudo -n` permission for install/symlink/systemctl steps
- `No restart` updates symlink/override without restarting `p2app.service`
- If a restart or override change leaves the local panel UI unusable but networking still works (e.g. Thetis continues to connect), use `/saturn/telemetry` from another device and run `Restore Unit Default`.
- Reasonable snapshot capture times:
  - `2 minutes` for post-change smoke checks
  - `10-15 minutes` for steady-state baseline comparisons
  - `30-60 minutes` for longer stability, underrun, or jitter investigations
  - `5 minutes` each for transition cases such as idle RX, active RX, TX, and disconnect/reconnect recovery
- When reviewing a captured snapshot, remember it is a single point-in-time sample plus page baseline summary; if the page reports hundreds of samples, that sample count comes from the browser baseline history rather than the raw `/p23_perf` payload itself.

### Update piHPSDR (Dedicated Terminal)

- Run `update-pihpsdr.py` from `/saturn/pihpsdr`.
- This page mirrors the dedicated terminal workflow (flags + SSE output) used by Update G2.
- In non-interactive web execution, backup prompts are skipped unless `-y` is selected.
- Installed build dependencies are reused. If packages are missing and privileged installation is unavailable to the web service, the updater stops once with a clear error instead of repeatedly attempting interactive `sudo` prompts.
- The updater applies Saturn's WDSP 2.00 Linux compatibility after each pull (and with `--skip-git`) so the renamed PureSignal `doPSCorrChange` worker and opaque Linux event handles build correctly.
- Output can be resumed from backend run logs (`/run_log`) after navigating away and back.
- On systems exposing non-UTF-8 stdout/stderr (for example `latin-1`), the updater now degrades unsupported status symbols instead of crashing; mirrored log files are written as UTF-8.

### Update deskHPSDR (Dedicated Terminal)

- Run `update-deskhpsdr.py` from `/saturn/deskhpsdr`.
- This page mirrors the dedicated terminal workflow (flags + SSE output) used by Update G2 and Update piHPSDR.
- If `~/github/deskhpsdr` does not exist and `--skip-git` is not selected, the updater clones the upstream deskHPSDR repo before the build step.
- In the normal channel, if the checkout already exists and `--skip-git` is not selected, the updater pulls `origin/<current-branch>` with `--ff-only` and auto-stashes local changes first when needed. If the checkout was left detached by the legacy GPIO channel, a normal run switches it back to `master` before pulling.
- The build step resolves helper scripts from the active Saturn repo root and then runs `scripts/deskhpsdr-test-build-on-current-image.sh --repo ~/github/deskhpsdr`.
- Select **Legacy GPIO V1 (deskHPSDR 2.6.84 / Trixie)**, or pass `--legacy-gpio`, for first-generation direct-GPIO controllers. This channel fetches and checks out the pinned upstream `2.6.84` tag, then requires `scripts/patches/deskhpsdr-libgpiod-v2.patch`. The patch ports input edge monitoring, line bias, and PTT/CW output requests to libgpiod v2 while preserving deskHPSDR's Controller1/Controller2-V1 mappings. The finished binary is rejected unless it advertises the `GPIO` build option.
- Before building any older deskHPSDR checkout that still includes `src/gpio.c`, the helper applies `scripts/patches/deskhpsdr-libgpiod-v2.patch` with `git apply` when the checkout still needs the local Saturn compatibility fix; if the patch is already present, the helper continues without error.
- The helper also applies `scripts/patches/deskhpsdr-active-receiver-init.patch`
  on every supported checkout. This prevents a null `active_receiver` crash
  when **Connect** starts Saturn XDMA before receiver construction finishes.
- Current upstream deskHPSDR removed the direct Raspberry Pi GPIO source path. For those checkouts, the helper skips the obsolete patch and builds `deskHPSDR` with `SATURN=ON` for the native G2/XDMA path.
- Do not combine `--legacy-gpio` with `--skip-git` unless the checkout is already exactly at tag `2.6.84`; the required-channel guard fails closed on any other source. To return to current deskHPSDR, clear the Legacy GPIO V1 checkbox and run the updater normally.
- The Trixie build probe proves the pinned source compiles and links against libgpiod v2. Final acceptance still requires a V1 panel: verify every encoder direction and push switch, all direct switches, PTT/CW inputs, and clean shutdown/restart before distributing the build.
- The helper keeps PulseAudio client libraries for build compatibility but prefers `pipewire-pulse` at runtime and removes the redundant `pulseaudio` daemon package when PipeWire Pulse is installed.
- `--no-install-deps`, `--no-clean`, and `--no-desktop-shortcut` map directly to the helper-script build flow.
- Unless `--no-desktop-shortcut` is selected, the helper installs direct
  `Type=Application` launchers in the application menu and on the Desktop.
- After the privileged prerequisite helper verifies the Debian packages, the build helper suppresses deskHPSDR's redundant internal `sudo apt-get` calls while it prepares the repo-local WDSP libraries. Other `sudo` commands retain their normal behavior.
- If a first build is interrupted, rerun with `--skip-git --no-install-deps --no-clean` to reuse the checkout, installed packages, prepared WDSP libraries, and completed object files.
- In non-interactive web execution, backup prompts are skipped unless `-y` is selected.
- On a fresh image, do not select `--skip-git`; otherwise the run fails because there is no local deskHPSDR checkout to build.
- Output can be resumed from backend run logs (`/run_log`) after navigating away and back.

### FPGA Flash (Dedicated Terminal)

- Run `flash_fpga.sh` from `/saturn/fpga`.
- The page invokes the writable runtime wrapper in `/opt/saturn-go/scripts`,
  which immediately hands off to the root-owned
  `/usr/local/lib/saturn-go/scripts/saturn-flash-fpga.sh` helper via `sudo -n`.
- The page discovers candidate images from `GET /get_fpga_images`.
- Use `Show only most current firmware` to limit dropdown selection to `latest_image` from backend scan.
- The script uses `sw_tools/load-FPGA/load-FPGA` with:
  - `-b <image>`
  - optional `-v` verify (enabled by default in UI)
  - optional `-f` fallback slot
- Flash is confirmation-gated (`--confirm FLASH` or short hash shown by script).
- In web mode, service-user passwordless sudo is required for hardware flashing.
- Output can be resumed from backend run logs (`/run_log`) after navigating away and back.

### Custom Scripts (Browser Managed)

- Use `/saturn/custom` to add/update/delete custom script entries from browser.
- Optional script content can be written directly to scripts directory.
- Custom script metadata is persisted in `SATURN_CUSTOM_SCRIPTS_FILE`.
- Default custom scripts are seeded by backend startup:
  - `cleanup-saturn-logs.sh`
  - `cleanup-saturn-backups.sh`
- Custom script output also resumes through `/run_log` buffering.

### Backup and Restore Scope

- Backup / Restore page now focuses on:
  - repo-root selection
  - portable settings download and transactional settings restore
  - source-repository download and transactional generation restore
  - installed immutable-release listing and download
  - repair pack and config verification
  - a notice directing whole-disk imaging to the local-console procedure

### Password Change

The current browser control is on **System → Custom Scripts**, in the output
card near the bottom of the page. Select **Change Password** there.

`POST /change_password` pipes the new password over stdin to `sudo -n saturn-admin-password.sh set`, which updates `/etc/nginx/.htpasswd` **and** the `SATURN_REMOTE_BASIC_AUTH` drop-in together, then schedules a deferred `saturn-go` restart. Audit alignment with `sudo /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh status`; recover a forgotten password from the console with `... reset` (see "Authentication layers" above).

The installer grants the service user sudoers entries for exactly `saturn-admin-password.sh set` and `status`; no direct `htpasswd` permission is needed.

Browsers that authenticated to the TLS listener (`/remote*`) hold a long-lived "remember this device" cookie, so operators type the password roughly once per browser. Changing the password invalidates every remembered device at the next request after the saturn-go restart; repeated wrong-password attempts from one IP are answered with growing delays (capped at 10s, forgotten after 15 minutes).

**Shared-device risk:** anyone who can use the same unlocked OS/browser
profile can operate Saturn Remote without re-entering the password for up to
one year. Use remembered login only in a trusted personal browser profile. If
a device is lost, transferred, or no longer trusted, change the administrator
password to invalidate all remembered devices, then clear Saturn site data and
cookies on that browser when possible. Per-device session listing and
revocation are deferred to REM-0602.

### Monitor and Process Control

- `monitor.html` polls `/get_system_data` every 1 second.
- process kill button calls `POST /kill_process/:pid` with CSRF header.
- backend blocks protected/root-owned process targets.

## Environment Variables

Service environment commonly used in deployment:

- `SATURN_ADDR` (default `127.0.0.1:8080`)
- `SATURN_WEBROOT` (default `/var/lib/saturn-web`)
- `SATURN_CONFIG` (default `$SATURN_WEBROOT/config.json`)
- `SATURN_SCRIPTS_DIR` (default `/opt/saturn-go/scripts`)
- `SATURN_LOG_DIR` (default `$HOME/saturn-logs`; installer-owned by the service user)
- `SATURN_STATE_DIR` (default `/var/lib/saturn-state`)
- `SATURN_REPO_ROOT_FILE` (default `$SATURN_STATE_DIR/repo_root.txt`)
- `SATURN_UPDATE_POLICY_FILE` (default `$SATURN_STATE_DIR/update_policy.json`)
- `SATURN_SATURNGO_UPDATE_POLICY_FILE` (default `$SATURN_STATE_DIR/saturngo_update_policy.json`)
- `SATURN_SATURNGO_DEPLOY_STATUS_FILE` (default `$SATURN_STATE_DIR/saturngo_deploy_status.json`)
- `SATURN_UPDATE_STATE_FILE` (default `$SATURN_STATE_DIR/update_state.json`)
- `SATURN_SNAPSHOT_DIR` (default `$SATURN_STATE_DIR/snapshots`)
- `SATURN_STAGING_DIR` (default `$SATURN_STATE_DIR/repo-staging`)
- `SATURN_CUSTOM_SCRIPTS_FILE` (default `$SATURN_STATE_DIR/custom_scripts.json`)
- `SATURN_PIHPSDR_ROOT` (default `$HOME/github/pihpsdr`)
- `SATURN_FPGA_DIR` (optional override for FPGA image scan path)
- `SATURN_RESTORE_MAX_UPLOAD_BYTES` (default `2147483648`)
- `SATURN_NGINX_CLIENT_MAX_BODY_SIZE` (installer default `64k`; ordinary routes)
- `SATURN_NGINX_RESTORE_MAX_BODY_SIZE` (installer default `2147549184`, the
  2 GiB restore payload plus 64 KiB multipart framing; restore routes only)
- `SATURN_WATCHDOG_URL` (default `http://$SATURN_ADDR/livez`)
- `SATURN_WATCHDOG_INTERVAL` (default `30s`)

`/livez` proves only that the Saturn Go process can answer requests. `/readyz`
checks the embedded release commit, mandatory state writability, configuration,
free disk space, and Saturn Bridge reachability while reporting P2 and XDMA
separately. Deployment uses
`/readyz?expected_commit=<full-40-character-commit>` so an old process cannot
validate a new staged release. `/healthz` remains a temporary compatibility
alias for `/livez` and should not be used for deployment validation.

## Troubleshooting

### Update G2 Cannot Write `saturn-logs`

If Update G2 exits before starting with `PermissionError` for a path under
`/home/pi/saturn-logs`, repair the standard appliance directory and retry:

```bash
sudo install -d -m 0755 -o pi -g pi /home/pi/saturn-logs
```

Current installers perform this repair automatically for the configured Saturn
Go service user. The Python updater also warns and continues with a private
temporary log if the preferred directory becomes unwritable; the terminal
output reports the fallback path.

### UI Loads, API Calls Fail

1. Check backend service status/logs.
2. Confirm NGINX proxy config is valid.
3. Confirm backend bind address matches NGINX proxy target.

### Script Runs Show No Output or Slow Output

- verify script exists and is executable in `/opt/saturn-go/scripts`
- check service logs for spawn errors
- check NGINX still has dedicated `/saturn/run` SSE location
- verify buffered lines are available:

```bash
curl -sS "http://127.0.0.1:8080/run_log?script=update-G2.py&from=0&limit=50" | jq
```

Live output uses a bounded 128-event channel. An `output backpressure` line
means the browser was slower than the helper; use `/run_log` for the retained
output. Resume output is capped at 1 MiB/5,000 lines and durable maintenance
output at 4 MiB/5,000 lines, with explicit truncation metadata/markers. Routine
scripts default to 30 minutes, update scripts to four hours, and no requested
deadline may exceed six hours. A deadline expiry is recorded as `timed_out` in
`/maintenance_jobs` after the process group is terminated. Tailscale mutations
use a ten-minute deadline.

### Restore Errors

Common causes:

- confirm token missing for non-dry-run restore
- archive too large for configured upload limit
- archive contains unsafe paths or unexpected top-level layout

### Appliance Update Errors

Common causes:

- invalid policy values (refs/owner/repo)
- remote fetch failures
- health check URL failure after staging
- insufficient disk space for snapshots/staging
- overlapping update actions (G2/appliance update/rollback) triggering `409 Conflict`

Check:

```bash
curl -sS http://127.0.0.1:8080/update_status | jq
ls -lah /var/lib/saturn-state/snapshots
ls -lah /var/lib/saturn-state/repo-staging
```

### Saturn Go Self-Update Errors

Common causes:

- Saturn Go policy repo URL not saved/invalid
- local repo has uncommitted changes and run omitted `--skip-git`
- `sudo -n` permissions missing for deploy copy/restart commands
- service restart happened, but browser did not reconnect yet

Check:

```bash
curl -sS http://127.0.0.1:8080/saturngo_deploy_status | jq
sudo cat /var/lib/saturn-state/saturngo_deploy_status.json
sudo systemctl status saturn-go.service
sudo journalctl -u saturn-go.service -n 200 --no-pager
```

### p2app Service Lab Errors

Common causes:

- `P2_app` source tree missing under active repo root
- build failure in `make`
- `sudo -n` denied for deploy/restart/revert (`install`, `ln`, `systemctl`)
- stale or unexpected systemd override contents from manual edits
- wrong front-panel detection mode after restart (try `Front panel mode = g2` or `g2v2` instead of `auto`)

Check:

```bash
curl -sS http://127.0.0.1:8080/p23_status | jq
sudo systemctl status p2app.service
sudo cat /etc/systemd/system/p2app.service.d/10-saturn-p23-switch.conf
ls -lah /opt/saturn-go/p23-apps
sudo /opt/saturn-go/scripts/p23-app-manager.sh --revert --verbose
```

### Main UI "Expected token '<'" / HTML Instead Of JSON

If the browser reports a JSON parse error but shows HTML content (for example a login page or an NGINX error page), the UI is receiving an HTML response from an API route such as `/custom_scripts`.

Check:

```bash
curl -i http://127.0.0.1:8080/custom_scripts
sudo systemctl status saturn-go.service
sudo journalctl -u saturn-go.service -n 100 --no-pager
```

Typical causes:

- `saturn-go.service` stopped/crashed (backend unavailable)
- reverse proxy/auth returned a login page or error page instead of JSON
- stale browser session/auth after service restart (refresh/login again)

### Verify Runtime File Set

```bash
curl -sS http://127.0.0.1:8080/verify_system_config | jq
```

### Maintenance Operation Is Busy

Saturn serializes only operations that claim the same host resource. The lock
owner is the maintenance broker/child, not Saturn Go, so restarting the web
service does not clear a legitimate busy condition.

Resource files are under `/run/lock/saturn-maintenance`:

- application deployment: `release`, `repository`, `package`, `radio`
- source/settings restore: `repository`, `radio`
- FPGA flash: `fpga`, `radio`
- Tailscale/network changes: `network`
- local-only disk work: `disk`
- diagnostics/log reads: shared `read-only`

Inspect without deleting or replacing a lock file:

```bash
sudo ls -la /run/lock/saturn-maintenance
sudo lsof /run/lock/saturn-maintenance/*.lock
ps -ef | grep '[s]aturn-maintenance-lock'
```

Do not remove a lock file to clear a busy response. Confirm whether its owner
is still doing useful work. Normal completion releases it automatically; an
abandoned broker can be terminated only after its maintenance child has been
identified and handled.

### Maintenance Job Was Interrupted Or Survived A Restart

Durable maintenance records and output are stored below
`/var/lib/saturn-state/maintenance-jobs`. Query the controller view with:

```bash
curl -sS http://127.0.0.1:8080/maintenance_jobs | jq
```

An `orphaned` job still has the same process identity and process group after a
Saturn Go restart. Its broker continues to hold the REM-0402 resource locks;
follow the record's `output_path` and wait for its atomic result before
retrying. An `interrupted` job has no surviving child and no completion result.
Review its durable output, verify the affected subsystem, and probe the needed
resource lock before retrying. Do not delete a record, result, or lock file to
force a new operation.

### Export Repair Pack

```bash
curl -sS http://127.0.0.1:8080/repair_pack -o saturn-repair-pack.tar.gz
```
