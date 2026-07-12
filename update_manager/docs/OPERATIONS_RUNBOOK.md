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
sudo bash update_manager/install_saturn_go_nginx.sh
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
- waits for backend health at `/healthz`

The normal installer does not require cloud-init to clone DSP sources, build
piHPSDR, or create the bridge service. Set `SATURN_INSTALL_BRIDGE=0` only for an
intentional backend-only installation. The default is fail-closed: Remote UI
assets are not considered a complete install if their matching bridge cannot
be built.

## Update Existing Deployment

After pulling repo changes, run installer again:

```bash
cd /home/pi/github/Saturn
sudo bash update_manager/install_saturn_go_nginx.sh
```

Installer is designed to refresh service, web assets, scripts, and config.
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

- `http://<host>/remote` should redirect to `https://<host>:8443/remote`.
- `http://<host>/saturn/remote` should redirect to `https://<host>:8443/remote`.
- `https://<host>:8443/remote` is the stable remote UI (`saturn-remote.html`).
- `https://<host>:8443/remote-next?phase42_split=1&phase44_tx_opus=1&phase44_tx_cfc=1&client_bust=bridgeprefill240-cfcessb3` is the current Saturn Remote checkpoint/default operator URL: Phase 42 split control/media sockets plus the guarded Phase 44 Opus TX path and the conservative ESSB CFC baseline. It serves `saturn-remote-next.html` + Vite bundle `saturn-remote-next.js` via `/remote-assets/remote-next.js`.
- `https://<host>:8443/remote-next` still serves the next-generation remote UI as a fallback entry point, and `/remote` remains the stable remote UI (`saturn-remote.html`). Both remote UIs share the same basic-auth credentials, `remote_settings.json`, and `remote_profiles.json` state.
- The current Phase 44 Opus TX processing checkpoint leaves Noise Gate available as an operator control but off by default, keeps TX EQ on with the ESSB-lite curve `3:+1,4:+2,5:+1,6:-1,7:+1,8:+3,9:+1`, and enables the conservative ESSB CFC baseline only when `phase44_tx_cfc=1` is present. Noise Gate can be explicitly started on for testing with `phase44_tx_noise_gate=1` and a guarded threshold such as `phase44_tx_noise_gate_db=-50`; once loaded, the operator Noise Gate toggle is allowed to persist instead of being repeatedly reset by the Phase 44 restore path.
- Field validation on 2026-06-11 confirmed `bridgeprefill240-gateoff1` transmitted clear Opus wideband TX audio from Chrome Android with `accepted=opus_wb`, `txNoiseGateEnabled=0`, `txMicDrops=0`, and `txUplinkHwm=854`.
- Field validation on 2026-06-11 also found the wider TX filter `50-4150` sounded good with `bridgeprefill240-gateoff2`; copy-log evidence showed `accepted=opus_wb`, `txMicDrops=0`, and `txUplinkHwm=637`.
- `https://<host>:8443/remote-next?phase42_split=1&phase44_tx_opus=1&phase44_tx_cfc=1&client_bust=bridgeprefill240-cfcessb3` enables the opt-in ESSB CFC baseline. The baseline keeps CFC conservative: precomp `1.0 dB`, CFC bands `50:+0.5, 100:+1.5, 200:+2.0, 500:+2.0, 1k:+1.0, 1.5k:+0.5`, and no CFC lift above 1.5 kHz. This is intended to add bottom-end warmth and warm mids without the older high-gain CFC profile that caused choppy TX audio. Optional `phase44_tx_cfc_precomp_db=<0..4>` can tune precomp for tests.
- Field validation on 2026-06-12 confirmed `bridgeprefill240-cfcessb2` held clear Opus wideband TX audio with CFC enabled: `accepted=opus_wb`, `codecDecodeFaults=0`, `codecPcmFallback=0`, `txMicDrops=15`, and `txUplinkHwm=4192` during a longer Chrome Android TX. The small browser drop count came from the Opus uplink hard cap and was not reported audible.
- Additional field validation confirmed the same CFC profile with cleaner short-run counters: `accepted=opus_wb`, `codecDecodeFaults=0`, `codecPcmFallback=0`, `txMicDrops=0`, `txUplinkHwm=596`, `iqStreaming=1`, and `audioStreaming=1`; operator report was "TX Audio is clear, working really well."
- Noise Gate validation confirmed the operator toggle and threshold controls persist under the Phase 44 restore path: `txNoiseGateEnabled=1`, `txNoiseGateThresholdDb=-35.0`, `accepted=opus_wb`, `codecDecodeFaults=0`, `codecPcmFallback=0`, `txMicDrops=0`, `txUplinkHwm=3318`, and the operator reported "Noise Gate working fine."
- Shared remote settings persist in `/var/lib/saturn-state/remote_settings.json`.
- Named remote Setup profiles persist in `/var/lib/saturn-state/remote_profiles.json`.
- The remote `Setup` menu supports profile save/load/delete, startup profile selection, and panadapter/waterfall display presets.
- The remote `Setup -> DSP` menu supports server-backed `NR` and `ANF` controls, including taps, delay, gain, and leakage.
- The remote meter panel now applies local smoothing for S-meter, TX power, and SWR so the analog gauges track like an operator console instead of stepping on raw samples.
- The TX power meter supports `Peak` and `Avg` display modes.
- Browser disconnect and `pagehide` now send an explicit TX-off command before the TCI websocket closes, and the bridge also forces TX/two-tone off when the client detaches.
- Two-tone test settings now persist in `remote_settings.json`, including `freq1`, `freq2`, `level`, `invert LSB-family`, and `tone 2 delay`.
- USB/LSB mode changes should move the transparent RX passband box to the correct side of center in both the panadapter and waterfall.

Remote Setup profile notes:

- `remote_settings.json` holds the active working state, including the current active profile name.
- `remote_settings.json` also carries the current DSP and TX test-control preferences used by the remote.
- `remote_profiles.json` holds the saved profile catalog plus the optional startup profile selection.
- A startup profile should be applied before opening a live phone session when you want a known panadapter, waterfall, and radio-control baseline.
- If the Setup menu opens underneath the panadapter after a deploy, confirm the latest `saturn-remote.html` (and, for `/remote-next`, `saturn-remote-next.html` + `saturn-remote-next.js`) was synced into `/var/lib/saturn-web/`.
- If USB/LSB signals or the transparent passband box appear on the wrong side of center after a deploy, confirm the latest `saturn-remote.html` (and, for `/remote-next`, `saturn-remote-next.html` + `saturn-remote-next.js`) was synced into `/var/lib/saturn-web/`.
- If `/remote-next` returns 404 on the bundle (`/remote-assets/remote-next.js`), confirm lockfile-only `npm ci && npm run build` succeeded in `update_manager/remote-web` and that `saturn-remote-next.js` plus `saturn-remote-next.js.sha256` are present in `/var/lib/saturn-web/`. The installer and `update-saturn-go.sh` now treat a missing bundle or checksum mismatch as a hard failure; this check covers manual or pre-promotion deploys.
- If TX appears stuck after a browser crash or tab close, confirm both `saturn-bridge.service` and `saturn-go.service` are on the latest deployed build with the explicit TX-release path.

## Secure Remote Access with Tailscale VPN

Tailscale is the recommended way to reach Saturn Remote from outside the LAN. It is **optional** — nothing in Saturn requires it, and the existing LAN entry points keep working unchanged. The deployment described below provides operator-friendly remote access (real Let's Encrypt cert, MagicDNS hostname, no port-forwarding) while preserving every Saturn security control: HTTP basic auth, RF TX opt-in, and loopback-only internal listeners.

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
Environment=SATURN_REMOTE_BASIC_AUTH=admin:<choose-a-strong-password>
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
printf '%s\n' 'new-password' | sudo /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh set --restart now
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

The installer writes `/etc/systemd/system/saturn-go.service.d/10-remote-auth.conf` (mode 0600 root:root) carrying `SATURN_REMOTE_BASIC_AUTH=admin:<password>` whenever a fresh password is captured during install (interactive prompt, `SATURN_ADMIN_PASSWORD` env, or non-TTY random generation). Reruns that reuse an existing `/etc/nginx/.htpasswd` preserve any pre-existing drop-in unchanged. If the installer cannot capture a fresh password and no drop-in exists, it warns the operator with the exact `systemctl edit` recipe to align the TLS path with the LAN nginx password.

The `/change_password` admin endpoint calls the privileged `saturn-admin-password.sh set` helper, which updates `/etc/nginx/.htpasswd` **and** the TLS auth drop-in together, then schedules a deferred `saturn-go` restart (~2s) so the TLS listener picks up the new credential. The two backends cannot drift through normal use. To audit or recover:

```bash
sudo /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh status   # sync_state=in_sync expected
sudo /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh reset    # console recovery: prints a fresh passphrase
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
```

### Restart

```bash
sudo systemctl restart saturn-go.service
sudo systemctl restart saturn-go-watchdog.timer
```

### NGINX Validation

```bash
sudo nginx -t
sudo systemctl reload nginx
```

### API Quick Checks

Through NGINX (authenticated session in browser) or locally against backend:

```bash
curl -fsS http://127.0.0.1:8080/healthz
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

- Download full backup from `backup.html` (or `GET /backup_full`).
- Validate archive first with restore dry-run (`POST /restore_full?dry_run=1`).
- Apply restore only after confirmation (`confirm=RESTORE`).
- For script-created directory backups, use Backup page "Script Backups" controls:
  - Saturn backups from `update-G2.py`: `GET /g2_backups`, `POST /g2_restore`
  - piHPSDR backups from `update-pihpsdr.py`: `GET /pihpsdr_backups`, `POST /pihpsdr_restore`

Important:

- restore overwrites active repo root using `rsync --delete`
- upload size is limited by `SATURN_RESTORE_MAX_UPLOAD_BYTES`
- non-dry-run full restore acquires the shared update lock; concurrent update actions return `409 Conflict`

### Clone SD Card to USB/SD Reader

Recommended workflow from `backup.html`:

1. Click `Refresh` and select the target device (for example `/dev/sdX`).
2. Optional: click `Wipe Target` to clear partition/signature metadata before cloning.
3. Click `Start Clone` and monitor progress/status/log output in the clone panel.

Notes:

- `Wipe Target` is a quick pre-clone wipe (metadata/signatures), not a full secure erase.
- The clone UI progress bar updates from `clone_pi_to_device.sh` progress messages (best results when `pv` is installed).
- Device detection uses block devices, not mounted filesystems.
- Use `lsblk` to verify the reader/card is detected; `df` will not show unmounted targets.
- Some USB readers only enumerate when the SD card is inserted before plugging in.
- If the dropdown is empty, check `dmesg -w`, `lsusb`, and `lsblk`.

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
- Output can be resumed from backend run logs (`/run_log`) after navigating away and back.
- On systems exposing non-UTF-8 stdout/stderr (for example `latin-1`), the updater now degrades unsupported status symbols instead of crashing; mirrored log files are written as UTF-8.

### Update deskHPSDR (Dedicated Terminal)

- Run `update-deskhpsdr.py` from `/saturn/deskhpsdr`.
- This page mirrors the dedicated terminal workflow (flags + SSE output) used by Update G2 and Update piHPSDR.
- If `~/github/deskhpsdr` does not exist and `--skip-git` is not selected, the updater clones the upstream deskHPSDR repo before the build step.
- If the checkout already exists and `--skip-git` is not selected, the updater pulls `origin/<current-branch>` with `--ff-only` and auto-stashes local changes first when needed.
- The build step resolves helper scripts from the active Saturn repo root and then runs `scripts/deskhpsdr-test-build-on-current-image.sh --repo ~/github/deskhpsdr`.
- Before building older deskHPSDR checkouts that still include `src/gpio.c`, the helper applies `scripts/patches/deskhpsdr-libgpiod-v2.patch` with `git apply` when the checkout still needs the local Saturn compatibility fix; if the patch is already present, the helper continues without error.
- Current upstream deskHPSDR removed the direct Raspberry Pi GPIO source path. For those checkouts, the helper skips the obsolete patch and builds `deskHPSDR` with `SATURN=ON` for the native G2/XDMA path.
- The helper keeps PulseAudio client libraries for build compatibility but prefers `pipewire-pulse` at runtime and removes the redundant `pulseaudio` daemon package when PipeWire Pulse is installed.
- `--no-install-deps`, `--no-clean`, and `--no-desktop-shortcut` map directly to the helper-script build flow.
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
  - full backup/restore
  - repair pack and config verification
  - Pi image and removable-device clone workflows

### Password Change

`POST /change_password` pipes the new password over stdin to `sudo -n saturn-admin-password.sh set`, which updates `/etc/nginx/.htpasswd` **and** the `SATURN_REMOTE_BASIC_AUTH` drop-in together, then schedules a deferred `saturn-go` restart. Audit alignment with `sudo /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh status`; recover a forgotten password from the console with `... reset` (see "Authentication layers" above).

The installer grants the service user sudoers entries for exactly `saturn-admin-password.sh set` and `status`; no direct `htpasswd` permission is needed.

Browsers that authenticated to the TLS listener (`/remote*`) hold a long-lived "remember this device" cookie, so operators type the password roughly once per browser. Changing the password invalidates every remembered device at the next request after the saturn-go restart; repeated wrong-password attempts from one IP are answered with growing delays (capped at 10s, forgotten after 15 minutes).

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
- `SATURN_MAX_BODY_BYTES` (default `2147483648`)
- `SATURN_RESTORE_MAX_UPLOAD_BYTES` (default `2147483648`)
- `SATURN_NGINX_CLIENT_MAX_BODY_SIZE` (installer default `2G`)
- `SATURN_WATCHDOG_URL` (default `http://$SATURN_ADDR/healthz`)
- `SATURN_WATCHDOG_INTERVAL` (default `30s`)

## Troubleshooting

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

### Export Repair Pack

```bash
curl -sS http://127.0.0.1:8080/repair_pack -o saturn-repair-pack.tar.gz
```
