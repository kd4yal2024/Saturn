# Changelog

All notable changes to `linuxdriver/xdma` are documented in this file.

## 2026-06-28

### Added
- Added a DKMS package template at `linuxdriver/dkms/dkms.conf`.
- Added `scripts/install-xdma-dkms.sh` to stage the supported XDMA source under
  `/usr/src/saturn-xdma-2020.1.8-saturn`, register it with DKMS, and build the
  module for a selected kernel.
- The DKMS installer disables the legacy manual kernel postinst hook by default
  after DKMS takes over, and supports `--uninstall` for rollback.

### Changed
- The DKMS installer refuses to reuse an already registered package/version
  unless `--force` is used or the caller bumps `SATURN_XDMA_DKMS_VERSION`.

## 2026-06-15

### Fixed
- Replaced legacy `EXTRA_CFLAGS` usage with Kbuild-supported `ccflags-y` so XDMA rebuilds survive newer Raspberry Pi/Trixie kernel header behavior.
- Updated `scripts/fix-xdma.sh` to pass one-off recovery build flags via `KCFLAGS` instead of `EXTRA_CFLAGS`.

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
