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

* `--work` must be new or empty. The Bridge's `perf.json`, readiness file and
  logs go there.
* Without `--wisdom` the Bridge plans its FFTs at start, about a minute.
  `--wisdom FILE` creates it on first use with the Bridge's own
  `--generate-fftw-wisdom` (about ten minutes on a fast x86 machine, once) and
  validates it every run. Wisdom is specific to the CPU.
* `--port 50001` makes it stand in for the Bridge behind the local proxy.
  That arrangement has not been exercised.

## Check it

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
