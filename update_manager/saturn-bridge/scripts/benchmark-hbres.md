# Pinned WDSP half-band resampler experiment

This is an isolated experiment, not a production patch. It reads WDSP commit
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
The production build script does **not** apply this candidate.

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

Before production integration, run on Cortex-A72 with the native compiler:

```bash
python3 update_manager/saturn-bridge/scripts/benchmark-hbres.py \
  update_manager/saturn-bridge/target-local/native-src/OpenHPSDR-wdsp \
  --cflags='-O3 -mcpu=cortex-a72'
```

This CPU-intensive benchmark can contend with live RX; schedule the ARM run
outside a quality soak. Do not infer whole-bridge savings from this isolated
kernel benchmark. ARM bit-equivalence and timing must pass before adding a
guarded build-time patch, then repeat full bridge regression and RF/audio
soak checks. The allocation shim uses calloc/free in place of WDSP allocation;
the timing excludes allocation and coefficient generation.
