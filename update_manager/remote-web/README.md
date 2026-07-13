# Saturn Remote Web

This directory contains the TypeScript runtime extracted from
`templates/saturn-remote-next.html`. Vite builds a single IIFE bundle at
`dist/saturn-remote-next.js`; the Remote TLS listener exposes it as
`/remote-assets/remote-next.js`, and the HTML consumes it through
`globalThis.SaturnRemoteNext`.

The bundle is required by `/remote-next`. DOM layout, WebGL spectrum/waterfall
rendering, AudioWorklet setup, and some browser event wiring remain in the HTML
template because they are tightly coupled to the live page.

## Source Layout

| Area | Responsibility |
|---|---|
| `src/audio/` | RX resampling, audio constants, and Opus TX encoding |
| `src/controller/` | Remote controller factory and controller types |
| `src/dsp/` | FFT and display calculations |
| `src/radio/` | Bands, frequencies, passbands, and DSP presets |
| `src/runtime/` | Browser session and storage runtimes |
| `src/settings/` | Defaults, normalization, and preference types |
| `src/state/` | Application state, preference application/export, and performance snapshots |
| `src/tci/` | TCI parsing, command generation, state, and message application |
| `src/transport/` | RX frames, split sockets, transport selection, Phase 42 adaptation, and TX uplink |
| `src/ui/` | Operator-display calculations such as meter math |
| `tests/` | Vitest unit, integration, smoke, and Phase 44 acceptance coverage |
| `scripts/validate-remote-next-layout.mjs` | Headless Chromium responsive-layout gate |

The automated suite covers unit, transport, controller, smoke, responsive
layout, TX-safety, and reconnection behavior. The production bundle is emitted
with a separate source map for diagnostics.

## Commands

Run commands from `update_manager/remote-web`:

```bash
npm ci
npm test
npm run typecheck
npm run build
npm run validate:remote-next-layout
```

The layout validator uses the current template and representative operator
state. It captures phone portrait/landscape, tablet, and laptop layouts—with
and without the navigation drawer—and fails on state-strip overlap, viewport
overflow, drawer overflow, overlapping groups, clipped text, or missing drawer
content. It complements, but does not replace, Safari/iOS device validation.

## Dependency Audit

The project has no npm production dependencies; its TypeScript, Vite, Vitest,
and Node type packages are build/test dependencies. Check deployable dependency
exposure with:

```bash
npm audit --omit=dev
```

As validated on 2026-07-12, that command reports zero vulnerabilities. A full
`npm audit` reports advisories in the Vite 5/esbuild development toolchain and
currently proposes a breaking Vite major upgrade. Do not run
`npm audit fix --force` as an unattended deployment step. Upgrade the build
toolchain separately, then rerun tests, type checking, the production build,
and the responsive-layout gate.

Run audit commands with `npm --prefix update_manager/remote-web ...` when your
shell is at the repository root. The repository root has no npm lockfile, so a
root-level `npm audit fix` correctly fails with `ENOLOCK`.

## Deployment

The supported appliance deployment path builds, verifies, copies, and restarts
the complete Saturn Go stack:

```bash
cd ~/github/Saturn
sudo bash update_manager/install_saturn_go_nginx.sh
```

The installer and `scripts/update-saturn-go.sh` both:

1. run the Remote web build;
2. generate `saturn-remote-next.js.sha256`;
3. copy the bundle, checksum, HTML, helper scripts, shared UI assets, and local
   fonts into `/var/lib/saturn-web`;
4. verify the deployed bundle checksum before completing.

For frontend development only, build locally and copy the bundle, checksum,
and HTML together:

```bash
npm ci
npm run build
sha256sum dist/saturn-remote-next.js >dist/saturn-remote-next.js.sha256
sudo install -m 0644 dist/saturn-remote-next.js dist/saturn-remote-next.js.sha256 /var/lib/saturn-web/
sudo install -m 0644 ../templates/saturn-remote-next.html /var/lib/saturn-web/
```

## Runtime Boundary

```text
Browser                         Saturn Go TLS              Saturn Bridge
saturn-remote-next.html  <--->  /remote-assets + /tci  <--->  TCI + WDSP + G2
        |
        +-- globalThis.SaturnRemoteNext (Vite IIFE bundle)
```

Saturn Go provides TLS/authentication, Remote assets, persisted settings and
profiles, and the WebSocket proxy. `saturn-bridge` owns the real-time TCI,
Protocol 2, audio/DSP, and radio-session boundary.

## Automatic Reconnection

`/remote-next` treats the Phase 42 control and media sockets as one supervised
session. If either lane fails, the paired lane closes and the browser retries
with exponential backoff and jitter. A connection is not considered recovered
until the bridge sends its TCI `ready` message; both socket-connect and
bridge-ready watchdogs prevent a half-open session from hanging indefinitely.

The supervisor pauses while the browser reports that the network is offline,
retries immediately when connectivity returns, and cancels all pending work
after **Go Offline**. After bridge readiness it restores the previous IQ and RX
audio choices, frequency, sample rate, and radio preferences. Transmit remains
fail-closed: MOX, PTT, microphone capture, and TX readiness are never restored
automatically.

## Future Refactoring

Keep pure calculations, transport state, preference handling, and reusable
browser runtimes in TypeScript. Move additional DOM/WebGL/AudioWorklet code only
when it can be tested without weakening the current fail-closed operator and TX
safety behavior. Any promotion of `/remote-next` to the stable `/remote` path
requires the full automated suite plus the Apple Safari validation runbook.
