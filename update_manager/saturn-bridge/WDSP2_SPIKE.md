# WDSP 2.00 Linux/ARM Spike

This branch keeps the production Saturn Bridge WDSP path unchanged while allowing
a local WDSP 2.00 archive to be built and selected explicitly.

## Build WDSP 2.00

The helper expects the TAPR WDSP repo to be cloned locally:

```sh
update_manager/saturn-bridge/scripts/build-wdsp2-linux-arm.sh
```

Defaults:

- WDSP 2.00 source: `/home/pi/github/OpenHPSDR-wdsp/wdsp 2.00/Source`
- piHPSDR Linux port shim source: `/home/pi/github/pihpsdr/wdsp`
- output archive: `update_manager/saturn-bridge/target/wdsp2-linux-arm/libwdsp.a`

Override those paths with `WDSP2_SOURCE_DIR`, `PIHPSDR_WDSP_DIR`, and
`WDSP2_BUILD_DIR`.

## Build Saturn Bridge Against WDSP 2.00

```sh
cd update_manager/saturn-bridge
SATURN_WDSP_DIR=target/wdsp2-linux-arm cargo build --release
```

The normal build still uses `/home/pi/github/pihpsdr/wdsp/libwdsp.a`.

## Current Limitation

WDSP 2.00 does not include piHPSDR's `RNNR` / `SBNR` symbols. Saturn Bridge now
detects those symbols at build time. If they are absent, NR3 and NR4 requests
fall back to EMNR so the bridge can link and run, but NR3/NR4 are not equivalent
to the current piHPSDR WDSP 1.29 behavior.
