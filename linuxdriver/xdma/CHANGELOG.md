# Changelog

All notable changes to `linuxdriver/xdma` are documented in this file.

## 2026-03-18

### Fixed
- Corrected async C2H buffer mapping in `cdev_sgdma.c` so read requests pin user pages with the proper DMA direction instead of using the H2C/write path semantics.
- Fixed async completion lifetime handling in `cdev_sgdma.c` so the shared `caio->cb` array is released once at final completion instead of freeing an individual callback pointer.
- Hardened async SGDMA setup in `cdev_sgdma.c` with allocation checks, zero-segment handling, safer `kcalloc()` allocation, and partial-submit accounting so early failures do not leak or queue malformed requests.

### Changed
- Updated `Makefile` clean rules to remove generated module metadata such as `.*.cmd` and `.*.d` so stale build products do not block subsequent rebuilds.
- Updated `.gitignore` to ignore generated dependency files from the kernel module build.

### Validation
- Rebuilt the module successfully with `make -C /home/pi/github/Saturn/linuxdriver/xdma -j2` on kernel `6.12.47+rpt-rpi-v8`.
- Deployed and reloaded the rebuilt module on the target system and verified the active module `srcversion` changed to `6D77120AF98D5FAC11DAC01`.
- Verified the supported XDMA recovery path in `/home/pi/github/Saturn/scripts/fix-xdma.sh` stops `p2app.service`, rebuilds the module, reloads `xdma`, and restores the service successfully.

## 2026-03-01

### Fixed
- Replaced disabled debug macros in `libxdma.h` with explicit `do { } while (0)` no-op macros to eliminate `-Wempty-body` warnings.
- Added missing prototypes for `xdma_kthread_start()` and `xdma_kthread_stop()` in `xdma_thread.h` to eliminate `-Wmissing-prototypes` warnings.

### Validation
- Rebuilt module with `make clean && make -j1` on kernel `6.12.47+rpt-rpi-v8`.
- Confirmed build completed with no compiler warnings.
- Loaded module with `modprobe xdma` and verified module presence via `lsmod`.
