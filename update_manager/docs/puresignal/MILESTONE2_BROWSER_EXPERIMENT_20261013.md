# PureSignal milestone 2 — real-browser transport experiment

2026-10-13. Isolated SilverForge worktree, base
`861f0fbc79e0e36c68a5c8ddc157e010b8be20df`.

## Decision

**Do not integrate the proposed four-packet banked Opus profile into production.**
Real Chrome produces five 20 ms packets for each four-input/80 ms bank after
flush. The current managed client correctly treats that fifth output as a
contract failure and requests release. Simply ignoring it is not an established
solution: the diagnostic four-packet truncation has larger boundary jumps and
more codec spectral leakage than continuous Opus.

Tagged PCM exercised the actual browser/Bridge/native owner in both backend
fixtures, including release/re-arm and stale-generation delivery. Continuous
Opus is a **reference outside the managed session**, not a fallback or an
approved replacement. Production remains Legacy and unchanged. PSG1 remains
explicitly opt-in; the experiment requires the exact capability advertisement
and codec acceptance before arming its offline fixture.

**A separate cancellation finding remains open.** One combined Direct replay
latched `TxAuthority` following ordinary cancellation; its reconnect settings
received explicit `GenerationMismatch` rejection. A deterministic test now
reproduces the fence/mailbox interleaving. Earlier and later combined browser
runs passed, but those passes do not close this race.

No G2 connection, transmission, FPGA, firmware, driver or polling changes. The
installed 1.31.002 / `0x53460004` image and GoldMarsh's tree are untouched. No
commit, push or deployment was performed.

## Actual path, sources and controlled inputs

```text
same local recorded speech + deterministic 700/1900 Hz tones
 -> AudioBufferSource -> MediaStreamDestination -> MediaStreamSource
 -> real AudioWorklet, fresh context/processor per generation
 -> PCM 512 samples OR real WebCodecs AudioEncoder, 960-sample inputs
 -> diagnostic raw corpus (all packets retained, including flush tail)
 OR opt-in PSG1 reference -> actual split control/media WebSockets
 -> actual TCI role/parser + RemoteMedia + existing shared FIFO
 -> actual shared OwnerLoop + one full native WDSP engine
 -> P2 loopback UDP OR Direct XDMA regular-file fixture
```

[Capture adapter](../../remote-web/src/puresignal/browser-capture-review.ts),
[browser experiment](../../remote-web/experiments/ps-browser.ts),
[unchanged managed reference](../../remote-web/src/transport/tx-generation-review.ts),
[existing Opus producer/config](../../remote-web/src/audio/tx-opus-encoder.ts),
[actual socket fixture](../../saturn-bridge/src/tci/ps_ingress_fixture.rs), and
[native/decoder replay](../../saturn-bridge/src/tx_owner_stop/scheduled.rs).

This is a real new **offline capture adapter**, not a mock AudioWorklet or
encoder. It does not migrate the installed inline microphone adapter. The
recorded source replaces the physical microphone; getUserMedia/device acoustic
behavior is not tested. Exact 48 kHz is required; no resampler silently carries
history. Partial blocks retain their original audio frame, and new generations
create a new context/processor. The observation clock conservatively includes
graph buffering rather than using main-thread receipt as sample time.

Input: first 9.6 s of local Downloads `KD4YAL_Ring_Introduction.wav`, mono,
peak normalized to 0.3, followed by 4.8 s of equal 700/1900 Hz tones. Each trial
uses identical 691,200 float samples (14.4 s at 48 kHz). Original source and
derived-input hashes are retained. Only this short transformed segment is in
the local review archive; no external upload or redistribution license is
asserted. Captured graph audio is also retained separately: graph delay is not
mistaken for codec delay or distortion.

Browser: actual Linux headless Chrome **153.0.8010.52**. Codec: 48 kHz mono,
24 kbit/s, voice/voip, 20 ms frames, complexity 5, FEC enabled, DTX disabled.
The accepted config/support result, every chunk timestamp/duration, source ages,
callback timing, CPU counters and encoded payload are saved. Host decoder:
libopus 1.3.1 (`f63e3597e52b57f8957bafb309e9f4d91108a3987136ace5f386f269a0947e55`).
The Bridge's **own** `TxCodecDecoder` independently replays the actual browser
corpus and checks every decoded sample against the listening artifacts.
Offline WDSP remains the pinned x86-64 archive
`8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`;
never install it on G2.

## Measurements and interpretation

| 14.4 s input | Tagged PCM | Continuous Opus reference | Four-input banked Opus |
| --- | ---: | ---: | ---: |
| Captured samples | 691,200 | 691,200 | 691,200 |
| Actual output packets | 1,350 | 721 | 900 |
| All decoded samples | 691,200 | 692,160 | 864,000 |
| Extra flush packets | 0 | 1 at final flush | 180: one per bank |
| Nominal diagnostic samples | 691,200 | 692,160 | 691,200 after explicit diagnostic omission |
| Estimated codec delay, cross-correlation | 0 ms | 6.44 ms | 6.44 ms |
| Aligned error SNR, recorded speech plus tones | 80.65 dB | 13.02 dB | 9.56 dB |
| 80 ms boundary absolute jump, p95, full-scale amplitude | 0.0220 | 0.0302 | 0.0847 |
| Tone leakage outside ±30 Hz fundamental windows, 100–4,000 Hz | −93.33 dB | −16.79 dB | −12.86 dB |

Banked **raw** audio is 18 s rather than 14.4 s: 25% more decoded samples.
The nominal banked listening file keeps the first four outputs of each real
encoder bank **only to diagnose the proposed four-packet receiver's sound**.
All fifth outputs remain in the raw file/corpus. No omission, pre-skip trimming,
sequence fabrication or fallback is applied in the actual managed path.

Codec latency is estimated against the actual captured samples, with search
limited to ±20 ms. It is not an RF-output or end-to-end networking latency.
Worklet-oldest-to-main p95 is about 29 ms for PCM and 39 ms for Opus; encoder
callback p95 is about 0.4 ms. Detailed maxima/percentiles are in per-mode JSON.
Chrome aggregate CPU was roughly 7–9% of one core per trial. This includes
the audio graph, encoder and Chrome process overhead, not isolated encoder CPU.
One fixed-order headless-host comparison does **not** establish statistical CPU
cost, mobile behavior, capture-device latency or production quality.

The spectral numbers describe **decoded audio codec leakage**, not RF IMD,
amplifier regrowth, PureSignal correction performance, or proof of unclipped RF
capture. Spectrum CSV uses Hz and full-scale-squared/Hz. No external RF spectrum
measurement occurred. Audio was preserved for listening; no subjective listening
qualification or perceptual score is claimed.

WebCodecs raw Opus does not convey additional priming-discard information in
each chunk. The [W3C Opus registration](https://www.w3.org/TR/webcodecs-opus-codec-registration/)
documents that raw decode without a description starts with zero pre-skip.
The measured extra flush packet must therefore be resolved by an explicit
tested framing/priming/history design, not hidden by discarding an unexpected
callback or retimestamping retained content.

## Actual browser lifecycle checks

Both fixtures run the same actual WebSocket handler and scheduling loop. The
recorded PCM control uses the previously reviewed −3 dB mic gain/EQ-enabled
settings, waits for applied receipts, and reaches Active in two fresh
generations with native rendering and actual adapter output. Initial exploratory
short/default-gain controls did not reach key qualification; they were not
counted as passes or used to relax the mic gate.

- Release/re-arm uses new generation tokens and fresh capture state. A retired
  frame is deliberately sent through the real media socket after re-arm.
- Reconnect creates a new control/media pair and reference client. An old-session
  packet is deliberately delivered to the next pair; token checks occur before
  codec interpretation, without making the old frame fresh.
- Undelayed banked Opus emits four packets per generation, then the measured
  fifth flush callback causes client release. There is no PCM fallback.
- The delayed experiment holds real encoded callbacks 500 ms, including across
  re-arm. Encoder IDs/timestamps establish that old callbacks really execute
  after the new offer. They send zero frames. Flush waits for delayed callback
  delivery; this is an explicit scheduling fault injection, not a fake codec.
- At most one native exchange occurs per owner turn. Completed controls leave
  queues empty and IQC off, with full P2 command completion or direct readback.
  These fixtures do not prove physical RF-off or a G2 timing bound.

Synthetic feedback keeps the offline native lifecycle advancing; it is not
sample-synchronized RF feedback from the recorded speech or a radio.

## New cancellation finding — reproduced, not repaired here

[Cancel::release](../../saturn-bridge/src/tx_thread/ps_owner_loop.rs) first closes
the output fence, then invalidates settings, then publishes the release mailbox.
[Lifecycle::tick](../../saturn-bridge/src/puresignal/tx_lifecycle.rs) services that
mailbox before [poll_health](../../saturn-bridge/src/tx_owner_stop.rs). If the
owner runs in the intervening window, poll_health sees expired authorization and
latches `TxAuthority` before the ordinary release is visible. Publishing release
later does not erase that fault. Output remains inhibited and cleanup progresses;
fresh arm/settings cannot bypass the latch.

The Direct browser failure's trace reaches `ReadyOff:Some(TxAuthority)` and its
next client's two settings receive queued followed by explicit rejected results.
The deterministic `observed_release_between_fence_and_mailbox_latches_authority_fault`
test pauses exactly at that ordering boundary with a healthy Active fixture,
checks no further output, completed cleanup, the retained fault and rejected
fresh arm. Its passing result **records the defect**, not its correction.

Next bounded owner work: represent ordinary cancellation coherently with the
fence/health check, add the deterministic inverse expectation, and rerun the
real-browser replay. Preserve immediate output inhibition, genuine authorization
faults, stop receipts and fault-latch rules. Do not blanket-ignore `TxAuthority`
or automatically acknowledge faults. No FPGA/driver/polling work is indicated.

## Executed verification and reproduction

Owner stubs: **46 library + 443 binary passed**, two existing ignored. New
cancellation reproducer: one passed with the unresolved behavior above.
Web targeted: **41 passed**; full worktree: **714 passed / 84 files**. Ordinary
web typecheck and strict experiment-file typecheck passed. Existing real-WDSP
combined regression suite: **7 passed**. Actual browser-corpus Bridge-decoder
test: one passed, all three modes matched saved audio. Combined actual-browser
test: passing executions on both fixtures **and a preserved failing Direct
execution**; not advertised as unconditional lifecycle qualification.

New browser integration and corpus tests are explicitly ignored in ordinary
test runs because they need Chrome, tool dependencies and local recording.
Formatting, whitespace and runner ShellCheck passed. Execution is SilverForge's
local evidence, not the independent reviewer's execution. Earlier packages and
their transcripts remain historical, not current browser qualification.

Install the pinned `remote-web/experiments/requirements.txt` into an isolated
Python tool environment; existing web npm dependencies and Chrome are required.
Supply a new result prefix and a local WAV of at least 9.6 s:

```sh
export PS_BROWSER_PYTHON=/path/to/experiment/python
export PS_BROWSER_SPEECH=/path/to/local-recorded-speech.wav
export PS_BROWSER_RESULTS=/tmp/new-ps-browser-result
bash update_manager/saturn-bridge/tests/run-puresignal-browser-experiment.sh \
  /path/to/verified-offline-wdsp-build
```

The wrapper runs comparison, actual Bridge decoder replay, then actual
ingress/native fixtures. A genuine lifecycle failure remains a failed run;
neither the runner nor the experiment clears its fault latch. Raw and nominal
decoded WAVs, captured inputs, spectra, JSON, original-age/packet traces and
owner events are preserved. No fixture discovers hardware or selects installed
startup paths.
