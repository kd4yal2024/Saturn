# Receive replay for the Saturn Bridge (no XDMA hardware)

Runs the **unmodified** `saturn-bridge` binary against a paced, repeatable
receive stream, so display and audio behavior can be compared with identical
input and without a radio. Receive only: RF TX is inhibited and the DUC device
is `/dev/null`. It does not open a second XDMA reader, because there is no
XDMA device at all.

## How it works

The Bridge already lets its three device paths be overridden by environment
(`SATURN_BRIDGE_XDMA_USER_DEVICE`, `_RX_DEVICE`, `_DUC_DEVICE`). The runner
points them at fake files and preloads a small shim; nothing in the Bridge
changes.

| Piece | What it stands in for |
|---|---|
| `make_register_file.py` | the FPGA register space: an ordinary file. Identity is the identified 1.31 image (`USR_ACCESS 0x53460003`, PCB 2). The DDC FIFO status register is a constant depth of 512 words, which makes the reader issue 4096-byte reads, about 844 a second, like the real device. Writes persist, so the Bridge's own read-back checks pass. |
| `xdma_replay_shim.c` (`LD_PRELOAD`) | the C2H DMA device. The reader uses `pread(fd, buf, n, 0)` and the real device ignores the offset, which a plain file cannot do, so the shim answers those reads with DDC frames in the exact wire format, paced in real time at 384 kS/s. It also answers the `SCHED_FIFO` calls, because the Bridge will not start its reader or TX thread without real-time scheduling and an unprivileged user cannot have it. |
| `run_replay_bridge.sh` | sets the environment and `exec`s the Bridge. |

The stream is either synthetic carriers (`--tones "offset_hz:dbfs,..."`, white
noise via `--noise-dbfs`, deterministic) or a raw DMA capture in the same frame
format (`--source FILE`, looped). Frames are 72 bytes: an 8-byte header
(`rate word 0x00100000`, marker byte `0x80`) and eight 8-byte sample words with
big-endian 24-bit I then Q.

## Run it

```sh
# Real native WDSP is needed for audio (see scripts/build-wdsp2-linux-arm.sh).
SATURN_WDSP_DIR=<wdsp build dir> SATURN_BRIDGE_WDSP_FLAVOR=wdsp2-2.10 \
  cargo build --release --locked

tools/replay/run_replay_bridge.sh \
  --bridge target/release/saturn-bridge \
  --work /tmp/replay-1 --port 50111 \
  --tones 1500:-30 --wisdom ~/.cache/saturn-replay/wisdom
```

* **TCP_NODELAY.** The Bridge now sets `TCP_NODELAY` on every TCI socket it
  accepts, unless `SATURN_BRIDGE_TCI_NODELAY=0` (so one binary serves both
  arms of a matched comparison). The shim reports the Bridge's own
  `setsockopt` on stderr, one line per call
  (`Bridge setsockopt(TCP_NODELAY=1) on fd N -> ok`), and the transport screen
  fails if the Bridge's setting and what it did disagree on any arm's
  connection. `SATURN_REPLAY_TCP_NODELAY=1` is a separate hook in the shim for
  a Bridge build that does not set the option itself (such as `main` before this
  change): the shim sets it on accept, reads it back, and logs
  `TCP_NODELAY enabled on accepted fd N (read back 1)`; the screen requires
  that on every arm when requested and none when not. (An unmerged
  `silverforge/rx-smoothness-g2` branch, commit `a90f33a`, adds
  `stream.set_nodelay(true)` to `handle_client`, and its 2026-10-02 trial
  document reports the browser-facing TLS socket result on a real G2; this
  Bridge change is the same five-line idea with the failure handling, a switch
  and tests.)
* `--work` must be new or empty. The Bridge's `perf.json`, readiness file and
  logs go there.
* Without `--wisdom` the Bridge plans its FFTs at start, about a minute.
  `--wisdom FILE` creates it on first use with the Bridge's own
  `--generate-fftw-wisdom` (about ten minutes on a fast x86 machine, once) and
  validates it every run. Wisdom is specific to the CPU.
* `--port 50001` makes it stand in for the Bridge behind the local proxy.
  That arrangement has not been exercised.

## Check it

The tests that start the Bridge, `tests/replay_e2e.rs` and
`tests/replay_transport_screen.rs`, share one harness (`tests/common/mod.rs`)
with these rules, which `tests/replay_harness.rs` checks with dummy children (no
Bridge, no WDSP; it runs in the ordinary `cargo test`):

* **Shutdown.** The harness sends SIGTERM to the *running* Bridge and requires it
  to exit with status 0, which is what the Bridge does (measured 2026-10-09). A
  nonzero exit, a death by SIGTERM, SIGSEGV, SIGABRT or SIGKILL, or an exit
  before the harness asked, fails the test.
* **Logs.** The Bridge's stdout and stderr go to files, never pipes, so a verbose
  Bridge cannot block on a full pipe while it is being measured.
* **Evidence.** Each run has its own directory (`$TMPDIR/saturn-replay-<pid>-<n>-<time>/`)
  with `logs/`, `work/` (perf.json, replay statistics) and, for the transport
  screen, `results/` (the table, every audio inter-arrival time, the final
  perf.json, the Bridge's path and SHA-256, the settings). **Every record is
  required:** a missing, unreadable or empty final perf.json, a Bridge hash that
  cannot be computed, an empty table, or an arm with no audio intervals fails
  the run instead of being skipped, and each file is read back after writing.
  The directory is deleted only
  if the test passed. It is kept on any failure, always for the transport
  screen (its records are the point), and for any run when
  `SATURN_REPLAY_KEEP=1`. The path is printed.

`tests/replay_e2e.rs` starts the Bridge through this runner and checks what a
client sees: raw IQ (rate, frame size, carrier level and mirror rejection),
WAN spectrum rows (rate, sequence continuity, carrier in the predicted bin),
RX audio (frames, continuity, a clean tone), and `perf.json` (no header errors,
resyncs, host-buffer drops, discontinuities or FIFO faults; about 844 reads/s;
the Stage A counters present). It is ignored by default because it needs the
real WDSP:

```sh
SATURN_WDSP_DIR=<wdsp build dir> SATURN_BRIDGE_WDSP_FLAVOR=wdsp2-2.10 \
SATURN_REPLAY_E2E_WISDOM=<wisdom file> \
  cargo test --release --locked --test replay_e2e -- --ignored --nocapture
```

## Limits — read before using a number from it

* **Timing is not the Pi's.** Real-time scheduling is faked, nothing is
  boosted, and the machine, CPU and load differ from a G2. Use it to prepare
  and debug, and to compare arms against each other on one machine, not to
  claim a Pi result.
* **Only the receive stream is emulated.** Read-to-clear and counter registers
  stay zero, the FIFO register never changes, so FIFO overflow, host-buffer
  loss and the FPGA telemetry paths are not exercised. A slow consumer simply
  falls behind a source that never overflows.
* **No transmit.** Do not use it to test TX behavior.
* **Sideband sense.** A positive-frequency synthetic carrier is shown above the
  center by the raw-IQ, spectrum-row and browser FFT paths, and the Bridge's
  WDSP demodulates it in LSB, not USB. This describes the fixture's sign
  convention in this build. Which sideband real hardware gives a station above
  the carrier is not established here and needs a known signal on a radio.
* **Safety.** The runner refuses to start if `/dev/xdma0_user` exists, so it
  cannot be run on a radio host by mistake (override only with
  `SATURN_REPLAY_ALLOW_RADIO_HOST=1`, which you should not need). It never
  opens `/dev/xdma*`.
* Listening smoothness still needs the real browser, AudioWorklet and proxy;
  a scripted client measures transport only.
