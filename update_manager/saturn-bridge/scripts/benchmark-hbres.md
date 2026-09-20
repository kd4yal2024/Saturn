# Pinned WDSP half-band resampler experiment

This is an isolated benchmark of the production index transformation. It reads WDSP commit
`584e8aca5ba1c4c6bc66fc0cc164ce567c8ba1e3` using Git, verifies the source SHA-256,
and compiles original and candidate implementations into a temporary benchmark.
It does not modify the native source checkout, link the bridge, access radio
devices, or install anything. Temporary generated sources/binary are removed
on exit. Requirements: Python 3, Git, a C compiler, libc and libm headers.

The candidate replaces only three circular-index modulo expressions. For
`0 <= ring_ptr < N`, `center=(N-1)/2`, and `1 <= j <= center`, each original
unwrapped index lies between `-(N-1)` and `N-1`. Adding N only when negative
therefore produces the identical index. No tap count, coefficient, summation
order, buffer size, rate, or phase changes. No unsafe floating-point flags.
The production build script now calls the shared `wdsp_hbres_index.py`
transformation on its build-directory copy only. Both input and output source
hashes are pinned. Unexpected sources fail before the existing archive is
removed; reapplying to already-patched source also fails. Benchmark and build
cannot silently use different transformations.

Run locally from the Saturn repository root:

```bash
python3 update_manager/saturn-bridge/scripts/benchmark-hbres.py \
  /mnt/c/Users/jd/github/OpenHPSDR-wdsp

python3 update_manager/saturn-bridge/scripts/benchmark-hbres.py \
  /mnt/c/Users/jd/github/OpenHPSDR-wdsp \
  --cflags='-O1 -g -fsanitize=address,undefined -fno-omit-frame-pointer'
```

The runner exhaustively checks ring indices for every pinned tap count.
The C harness compares output bits and internal ring state over 80 supported
rate/block-size/in-place combinations, including bypass, with silence,
impulse, near-full-scale DC, independent I/Q random samples, a low-frequency
tone, and a high-frequency tone. It flushes between signal cases and processes
32 consecutive blocks per case. Timing covers the 384 kHz to 48 kHz cascade
with a 2048-pair input block (256-pair DSP block), alternating implementation
order across four trials. Assertions must remain enabled.

To repeat the native Cortex-A72 gate with the native compiler:

```bash
python3 update_manager/saturn-bridge/scripts/benchmark-hbres.py \
  update_manager/saturn-bridge/target-local/native-src/OpenHPSDR-wdsp \
  --cflags='-O3 -mcpu=cortex-a72'
```

Keep `wdsp_hbres_index.py` alongside the benchmark script when copying it.
This CPU-intensive benchmark can contend with live RX; schedule the ARM run
outside a quality soak. Do not infer whole-bridge savings from this isolated
kernel benchmark. The native gate passed at 1.84x kernel speedup; deployment
still requires matched full-bridge CPU and RF/audio soak checks. The allocation
shim uses calloc/free in place of WDSP allocation;
the timing excludes allocation and coefficient generation.

## Controlled deployment and soak

Use the integration commit supplied with the deployment handoff, not the
earlier benchmark-only commits. Push that commit to
`v30-bridge-fifo-integration` before fetching it on G2. Keep RF TX disabled.
Check the worktree is clean, fetch the branch, verify the supplied commit is
an ancestor of its remote tip, and check out that commit detached.

Before running the normal installer, preserve the currently installed binary:

```bash
rollback_dir=$(mktemp -d /home/pi/saturn-bridge-rollback.XXXXXX)
cp /opt/saturn-go/bin/saturn-bridge "$rollback_dir/saturn-bridge"
cp /run/saturn-bridge/perf.json "$rollback_dir/perf-before.json"
sha256sum "$rollback_dir/saturn-bridge"
echo "Keep this rollback directory: $rollback_dir"

sudo env SATURN_USER=pi \
  SATURN_REPO_ROOT=/home/pi/github/Saturn-v30-telemetry-fix \
  SATURN_BRIDGE_RF_TX_ENABLED=0 \
  bash update_manager/scripts/install-saturn-bridge.sh
```

The installer rebuilds WDSP and the bridge and interrupts RX when activating
the binary. Its output must include `hbres-index-wrap-v1 applied` and a passed
runtime check. Confirm the running `/run/saturn-bridge/perf.json` build Git SHA
matches the supplied integration commit and `build_git_dirty` is false. Check
service status/restarts and compare installed, built, and running executable
hashes. Firmware remains V30; WDSP remains pinned to 2.00.

Resume the same station, mode, rate, DSP options, LAN transport, and browser
workload used before deployment. Let startup settle, record a fresh baseline,
and compare counter deltas rather than totals from the old process. Start
with a short check, then soak. Acceptance requires fewer CPU cycles/time at
the same workload, unchanged IQ throughput, no new IQ/host/audio drops,
no added discontinuities/FIFO faults, and normal reception, tuning and audio.
This optimization must not be accepted on CPU reduction alone.

If reception regresses, use the saved rollback directory (the shell variable
above survives only in the shell where it was assigned):

```bash
test -f "$rollback_dir/saturn-bridge" && (
  set -e
  sudo systemctl stop saturn-bridge.service
  sudo install -m 0755 -o root -g root \
    "$rollback_dir/saturn-bridge" /opt/saturn-go/bin/saturn-bridge
  sudo systemctl start saturn-bridge.service
  systemctl is-active saturn-bridge.service
)
```

WDSP is statically linked into this binary, so binary rollback also restores
the prior resampler. Do not enable P2app alongside the Direct-XDMA bridge.
