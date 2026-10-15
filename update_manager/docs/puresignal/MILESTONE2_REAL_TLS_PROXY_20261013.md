# Milestone 2 — actual Saturn Go TLS/proxy executable, offline fixtures

2026-10-13. Isolated SilverForge worktree, base
`861f0fbc79e0e36c68a5c8ddc157e010b8be20df`. Uncommitted; no push or deployment.

## Outcome

The actual **Saturn Go executable**, not the previous Python proxy mirror, now
passes the managed tagged-PCM full-page/native replay on both offline backend
fixtures. Authenticated access, real control/media proxy pairing, settings
rejection, ordinary release/re-arm and keyed upstream connection loss/reconnect
are exercised through its real HTTPS/WSS listener.

No production fix or enablement was required. Saturn Go source, the Bridge
production owner, client controller and template are unchanged by this slice.
Added/changed files are test harnesses, a test wrapper and review packaging/docs.
The shared managed startup path remains inhibited/default Legacy; browser
selection remains opt-in/off by default. No G2, firmware, FPGA, kernel driver,
RXC1 polling or GoldMarsh worktree change occurred. This is not live TX or RF
PureSignal qualification.

## Actual executed path

```text
real Chrome + actual Saturn Remote template/IIFE
    HTTPS /remote-next -> real normal-entry redirect/default query
    Basic login -> real secure remember-device cookie
    fresh browser context: cookie only, no Basic credentials
    WSS /saturn/control?session=... + /saturn/media?session=...
        -> unmodified saturn-go executable: auth/origin/admission/router
        -> real upstream /control + /media and session_lane initialization
        -> actual Bridge socket handler + shared ingress
        -> existing one OwnerLoop + full native WDSP
        -> P2 loopback UDP OR Direct XDMA ordinary files (no hardware)
        -> matching stop receipts, native cleanup, fresh authorized re-arm
```

Source links:
[Go executable startup](../../rust-server/src/main.rs),
[TLS authentication/router/proxy](../../rust-server/src/remote_tls.rs),
[real-executable launcher](../../remote-web/experiments/real_saturn_go_proxy.py),
[actual browser runner](../../remote-web/experiments/run_managed_pcm_ui.py),
[real Bridge fixture](../../saturn-bridge/src/tci/ps_ingress_fixture.rs),
[combined native replay](../../saturn-bridge/src/tx_owner_stop/scheduled.rs), and
[offline test entry](../../saturn-bridge/tests/run-puresignal-real-proxy.sh).

The executable was built locally with `cargo build --locked --offline` against
the unchanged Saturn Go source and lockfile. Executed binary SHA256:

```text
cefbb7530e018929f0f36626cd2020d8db424f15d834de85e809b1e58b6a8a24
```

That hash identifies this offline x86 debug build, not an installed G2 binary.
Go source, Cargo files and all five `include_str!` inputs are included for a
source rebuild. The executable is not a deployment payload and is not packaged.

Each backend uses a separate Go process with ephemeral **loopback-only** HTTP
admin/TLS addresses and explicit loopback Bridge upstream. All inherited
`SATURN_*` settings are removed before supplying new temporary state, webroot,
scripts and repository-pointer paths. Startup persistence and UI settings writes
therefore use isolated state, not appliance files. No admin maintenance action
is requested. Browser network requests are restricted to loopback.

The Go process generates its real self-signed certificate. Python probes trust
that exact certificate and verify the `127.0.0.1` hostname; both final backends
negotiated **TLS 1.3 / TLS_AES_256_GCM_SHA384**. Chrome uses
`ignore_https_errors` in its isolated context for that test certificate. This
checks the actual TLS executable path, not deployment CA trust or certificate
distribution. Private certificate keys, generated passwords, cookie secrets and
cookie values are retained only in private temporary runtime directories and
are **not included in the review archive**.

## Authenticated access and pairing evidence

Final run: `real-proxy/final-default-cookie-{P2,DirectXdma}/`.
Each backend records **25 checks and 10 proxy-capacity snapshots** in
`real-proxy-results.json`, plus Go logs and raw browser WebSocket frames/paths.

| Real runtime check | Observed result on both backends |
| --- | --- |
| Authenticated `/remote-next` entry | 307 to the real default `transport=split&tx_opus=1&tx_cfc=1` query; browser follows it |
| Anonymous page, bundle and metrics; wrong page password; forged cookie | 401 |
| Anonymous control and media WebSocket upgrade | 401 |
| Authenticated cross-origin control/media upgrade | 403 |
| Authenticated missing split-session ID | 400 |
| Authenticated page and bundle bytes | 200, hashes match actual template/built IIFE |
| Cookie-only page and actual fresh-context browser WSS | accepted; Go logs explicitly say `accepted via cookie auth` |
| Remember-device cookie attributes | Secure, HttpOnly, SameSite=Strict |
| Paired control/media per connection | exactly one authenticated client and two active connections |
| Duplicate control/media for that active session | 409; no new slot/upstream connection |
| Disconnect and final cleanup | zero active proxy clients/connections after each connection |
| Go process cleanup | SIGTERM graceful shutdown, exit 0, no forced kill |

Every actual UI connection has matching control/media session IDs. Recorded
binary TX PCM frames are sent on the media lane; recorded text commands are on
the control lane. Successful native IQ output is the non-vacuous positive
control that the real proxy's upstream lane markers reach the real Bridge
pairing/ingress path. Denied upgrades leave zero proxy sessions before the
browser begins. Capacity high-water marks remain one client/two connections;
six duplicate-lane and two missing-session rejections are expected probes, not
transport loss. The existing broader client-capacity policy is not exhaustively
requalified here.

Basic authentication is also exercised by the earlier actual-executable browser
run, retained as `real-proxy/basic-browser-*`. A second cookie-only run with an
explicit query is retained as `real-proxy/earlier-cookie-*`; the latest default
entry run is the current evidence. All three passes use the same Go binary.
The normal default query's legacy Opus flags do **not** change the managed
advertisement: the runner requires PCM-only advertising, with no Opus fallback.
Banked Opus remains stopped; continuous legacy Opus remains separate.

## Settings and TX lifecycle evidence

The actual gain control and EQ button send separate settings requests through
the proxy and commit both. Each of three browser connections then makes two
fresh TX actions. Normal release returns to clean readiness before another arm.
During the second connection's second Active generation, the existing backend
fixture deliberately resets its **upstream Bridge sockets**. The real Go proxy
propagates the loss to the browser, the owner cleans up, and the UI reconnects
through authenticated WSS, reapplies fresh settings and authorizes new TX.

This is keyed upstream loss through the actual proxy, not a simulated direct
call to `ManagedPcmSession`, and not a claim that proxy-process death, downstream
media-only loss or every possible disconnect ordering has been exercised.
Expected reset/protocol warnings in the logs are preserved and distinguished
from unexpected owner faults.

After the six Active generations, the real UI proposes 6 W against the offline
fixture's 5 W admission ceiling while unkeyed. The real Bridge returns:

```text
tx_config:0,7,rejected,InvalidConfig,0;
```

There is no queued receipt for that ID and no configuration commit. Through the
actual proxy the UI shows:

```text
Settings 7: rejected (InvalidConfig); before queueing · 0 pending
```

That exact result and its reason remain after **5.2 seconds**. Rejection retains
output inhibition. The 6 W value is a rejected settings proposal, not measured
or transmitted RF power.

| Latest default-entry run | P2 loopback fixture | Direct XDMA file fixture |
| --- | ---: | ---: |
| Authenticated paired UI connections | 3 | 3 |
| Fresh Active generations | 6 | 6 |
| Committed configurations | 7 | 7 |
| Microphone blocks processed | 387 | 399 |
| Fixture IQ packet writes | 1,609 | 1,630 |
| Deadline stops | 0 | 0 |
| Primary/authority/native-cleanup fault observed | none | none |
| Final stop/native cleanup; empty queues; IQC off | complete | complete |
| Parallel legacy TX output | none | none |

The loop retains its assertion of at most one native exchange per owner turn.
Native feedback is synthetic fixture input and output uses loopback/regular
files. Counts are not RF performance, FPGA/DMA-loss counters or G2 latency
bounds. The owner `rejected` counts include obsolete/cancelled fixture work and
are not advertised as zero losses.

## Executed checks and reproduction

- Actual Go build: locked/offline, unchanged production source.
- **154 Go tests passed**, including the **34 TLS/proxy tests** also run as a
  focused subset (the counts are not additive).
- Real Go/Chrome/actual-Bridge/full-WDSP replay: **three passes, each covering
  both backend fixtures**. Latest default-entry pass: **35.81 seconds**.
- **742 web tests** and client typecheck passed; managed controller and template
  are unchanged from the accepted pre-queue correction.
- Test wrapper shell syntax/ShellCheck, Python syntax and diff-whitespace checks
  passed. Earlier native/owner-adapter evidence remains historical; not relabeled
  as a new full owner suite run in this slice.

Run `cargo build --locked --offline` in `update_manager/rust-server` and build the
actual web IIFE. Then use the existing explicit native/browser inputs with:

```bash
PS_SATURN_GO_BIN=/absolute/path/to/locally-built/saturn-go \
  bash update_manager/saturn-bridge/tests/run-puresignal-real-proxy.sh \
  /absolute/path/to/verified-x86-wdsp-build
```

Also supply `PS_REMOTE_TEMPLATE`, `PS_REMOTE_BUNDLE`, `PS_BROWSER_PYTHON`,
`PS_BROWSER_RUNNER`, `PS_BROWSER_SPEECH`, `PS_BROWSER_RESULTS` and the isolated
Python dependency path. Use fresh output prefixes. The wrapper enables the
pre-queue rejection case; only the offline fixture enables the shared owner.
Do not launch the installed Bridge or install these x86 artifacts on G2.
The short local speech recording remains private review material, with no
assertion of public redistribution rights.

The pinned x86 WDSP archive/source checks remain
`8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
That is not the installed native-library identity.

For a source rebuild **from the extracted archive rather than a Git checkout**,
set `SATURN_BUILD_COMMIT=861f0fbc79e0e36c68a5c8ddc157e010b8be20df` when running
Cargo. `build.rs` otherwise records `unknown` without Git metadata, and
`health::tests::release_identity_requires_full_matching_commit` correctly
rejects it. The initial extraction run failed only that identity assertion;
the explicit-identity reconstruction passed all 154 tests. Both transcripts
are retained. This is a build-input requirement, not a proxy/runtime failure
or a weakened readiness test. No source/test gate was changed to obtain a pass.

## Remaining review boundary

The runtime proxy substitution boundary is now checked through the actual Go
executable **offline**. Production managed selection is still disabled; this
package does not enable or qualify live PureSignal. Installed-library identity,
RXC1 feedback topology, live admission thresholds, RF-off verification, overload
interval coverage, G2 timing and RF IMD/linearity remain separate hardware-review
work. No firmware, driver or polling change follows from this result.
