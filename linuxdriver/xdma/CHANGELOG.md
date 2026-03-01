# Changelog

All notable changes to `linuxdriver/xdma` are documented in this file.

## 2026-03-01

### Fixed
- Replaced disabled debug macros in `libxdma.h` with explicit `do { } while (0)` no-op macros to eliminate `-Wempty-body` warnings.
- Added missing prototypes for `xdma_kthread_start()` and `xdma_kthread_stop()` in `xdma_thread.h` to eliminate `-Wmissing-prototypes` warnings.

### Validation
- Rebuilt module with `make clean && make -j1` on kernel `6.12.47+rpt-rpi-v8`.
- Confirmed build completed with no compiler warnings.
- Loaded module with `modprobe xdma` and verified module presence via `lsmod`.
