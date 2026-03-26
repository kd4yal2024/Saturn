# Saturn LinuxDriver / XDMA Handoff

Date: 2026-03-26

## Purpose

This is an internal working handoff note for the Saturn Linux XDMA driver
layer.

It documents what the `linuxdriver` tree tells us about:

- driver provenance
- kernel compatibility expectations
- device-node creation
- rebuild/update workflow
- where Linux driver failures stop and FPGA / PCIe failures begin

This note is intended to complement the FPGA / Protocol 2 handoff, not replace
it.

## Primary Sources Reviewed

- `linuxdriver/readme.txt`
- `linuxdriver/xdma/readme.md`
- `linuxdriver/xdma/Makefile`
- `linuxdriver/xdma/version.h`
- `linuxdriver/xdma/CHANGELOG.md`
- `linuxdriver/xdma/xdma_mod.c`
- `linuxdriver/xdma/libxdma.c`
- `linuxdriver/xdma/cdev_ctrl.c`
- `linuxdriver/tests/load_driver.sh`
- `linuxdriver/etc/udev/rules.d/60-xdma.rules`
- `linuxdriver/etc/udev/rules.d/xdma-udev-command.sh`
- `FPGA/create_saturn_project.tcl`

Related companion notes:

- `docs/protocol2/Saturn_FPGA_Protocol2_Handoff.md`
- `docs/protocol2/Saturn_vs_openHPSDR_v4.4.md`

## Executive Summary

The `linuxdriver` tree is a Saturn-maintained fork of the Xilinx XDMA
out-of-tree driver, not an in-tree kernel driver and not a DKMS package.

The most important conclusions are:

- Saturn depends on a manually rebuilt XDMA kernel module per kernel version.
- The supported operational path is `scripts/fix-xdma.sh`, not ad hoc manual
  rebuilds.
- The current active driver already contains Raspberry Pi and newer-kernel
  compatibility changes.
- The Saturn FPGA PCIe device ID `10ee:7024` is already supported by the
  driver.
- If the FPGA endpoint does not enumerate on PCIe, the Linux driver cannot fix
  that. No `/dev/xdma*` nodes will appear because there is nothing to bind to.

The practical stack model is:

1. FPGA endpoint must appear on PCIe.
2. XDMA module must build and load for the running kernel.
3. `/dev/xdma*` nodes must appear.
4. `P2_app` / `P3_app` can start.

## What The LinuxDriver Tree Is

The `linuxdriver` tree contains:

- `xdma/`
  - the current XDMA kernel-module source used on modern Raspberry Pi OS
- `xdma_pre_kernel_5.18/`
  - an older compatibility copy for earlier kernels
- `tools/`
  - user-space XDMA test and register-access utilities
- `tests/`
  - shell scripts that load the module and generate basic test traffic
- `etc/udev/`
  - rules and helpers for friendly `/dev/xdma/cardN/*` symlinks

This is fundamentally a maintained fork of the Xilinx reference XDMA driver.

## Driver Identity And Provenance

From `linuxdriver/xdma/version.h`:

- driver version is still labeled `2020.1.8`

From `linuxdriver/xdma/xdma_mod.c`:

- module name is `xdma`
- module description is `Xilinx XDMA Reference Driver`

So the right mental model is:

- "Xilinx XDMA 2020.1.8 with Saturn / Raspberry Pi compatibility fixes"

not:

- "a brand-new Saturn-specific DMA driver"

## Saturn / Raspberry Pi Specific Changes

The `linuxdriver/readme.txt` file documents two important Raspberry Pi related
fixes:

### 1. 64-bit BAR mmap Addressing

In `cdev_ctrl.c`, `bridge_mmap()` uses 64-bit variables for:

- BAR offset
- BAR physical address
- virtual size
- physical size

Reference:

- `linuxdriver/xdma/cdev_ctrl.c`

This is important because the Raspberry Pi memory / PCIe environment can expose
physical addresses that do not fit safely in 32-bit temporary variables.

### 2. 64-bit DMA Mask Handling

In `libxdma.c`, the driver now uses:

- `dma_set_mask()`
- `dma_set_coherent_mask()`

Reference:

- `linuxdriver/xdma/libxdma.c`

This is part of making the XDMA path behave correctly on Pi DMA addressing.

### 3. Newer Kernel Compatibility

The current `xdma/` tree already contains newer-kernel compatibility work, for
example:

- `vm_flags_set()` handling in `cdev_ctrl.c`

There is also a separate `xdma_pre_kernel_5.18/` tree for older kernels.

Implication:

- Saturn is already maintaining kernel-version-specific compatibility within the
  driver source tree

## The Supported Operational Workflow

The docs in:

- `linuxdriver/readme.txt`
- `linuxdriver/xdma/readme.md`

both point to the same preferred workflow:

- `sudo bash /home/pi/github/Saturn/scripts/fix-xdma.sh`

The intended role of that script is:

- install matching kernel headers when needed
- stop `p2app.service`
- rebuild and reinstall `xdma.ko`
- reload the module
- restart `p2app.service`
- pre-stage the module for a newer installed kernel of the same Pi flavor when
  appropriate

This matters because the driver is not self-maintaining across kernel updates.

## This Is Not DKMS

The `linuxdriver/xdma/Makefile` is a conventional out-of-tree kernel-module
build:

- `make -C $(KDIR) M=$(PWD) modules`
- `modules_install`
- `depmod`

There is no DKMS packaging in this tree.

Implication:

- kernel updates can break XDMA availability until the driver is rebuilt for the
  new kernel
- this explains why OS updates can leave a previously working system without
  `/dev/xdma0_user`

## Device Identification

From `linuxdriver/xdma/xdma_mod.c`:

- the driver already matches a large set of Xilinx PCIe device IDs
- Saturn's actual FPGA device ID `10ee:7024` is included

From `FPGA/create_saturn_project.tcl`:

- the PCIe endpoint in the FPGA project is configured with device ID `7024`

This is an important diagnostic boundary:

- if `lspci` does not show the Xilinx endpoint, the problem is not "missing PCI
  ID support in the Linux driver"

The driver already knows about the correct Saturn endpoint ID.

## Device Nodes And Udev Behavior

The kernel driver creates the canonical device nodes under `/dev`, such as:

- `/dev/xdma0_user`
- `/dev/xdma0_h2c_0`
- `/dev/xdma0_c2h_0`

Then udev adds friendlier symlinks using:

- `linuxdriver/etc/udev/rules.d/60-xdma.rules`
- `linuxdriver/etc/udev/rules.d/xdma-udev-command.sh`

This creates paths such as:

- `/dev/xdma/card0/user`
- `/dev/xdma/card0/h2c0`
- `/dev/xdma/card0/c2h0`

Implication:

- Saturn app code may reference either the kernel-created node or the udev
  symlink, but both depend on successful driver binding first

## What "Driver Working" Actually Means

At the `linuxdriver` layer, "working" means all of the following are true:

1. the FPGA endpoint enumerates on PCIe
2. the `xdma` module loads successfully
3. the module binds to the Xilinx PCIe endpoint
4. `/dev/xdma*` nodes appear
5. register access through the `user` / control interface works
6. optional DMA test traffic works with the supplied tools

This is narrower than "the radio is fully operational," but broader than "the
module compiled successfully."

## Most Important Diagnostic Boundary

This tree makes one recurring field issue much clearer:

### If PCIe Endpoint Enumeration Fails

Examples:

- boot log says `brcm-pcie ... link down`
- `lspci` does not show `10ee:7024`
- no Xilinx endpoint is visible at all

Then:

- no Linux driver can bind
- no `/dev/xdma0_user` node will appear
- `P2_app` / `P3_app` startup will fail no matter how well the driver compiles

This is not a Linux XDMA compile problem.

It is upstream of the driver:

- PCIe link training
- FPGA endpoint availability
- reboot / warm restart behavior
- board reset or power timing

### If PCIe Enumeration Succeeds But XDMA Fails

Examples:

- `lspci` shows `10ee:7024`
- module fails to build, load, or bind
- `/dev/xdma*` nodes do not appear despite endpoint presence

Then the problem is in the Linux driver layer:

- missing headers
- kernel API mismatch
- stale module build
- install / depmod / modprobe problem
- udev rules not in place

This is the boundary we need to keep clear in field support.

## Test And Utility Layer

The tree includes basic user-space tools:

- `tools/reg_rw`
- `tools/userio_rw`
- `tools/dma_to_device`
- `tools/dma_from_device`
- `tools/performance`

And basic test scripts:

- `tests/load_driver.sh`
- `tests/run_test.sh`
- `tests/dma_memory_mapped_test.sh`
- `tests/dma_streaming_test.sh`

These are useful only after:

- the endpoint enumerates
- the module loads
- device nodes exist

They do not help with a "link down / no endpoint" condition.

## Current Maintenance State

From `linuxdriver/xdma/CHANGELOG.md`:

- the driver has been actively maintained in 2026
- recent work includes:
  - async SGDMA buffer-direction fixes
  - callback lifetime fixes
  - allocation hardening
  - build-cleanup improvements

Implication:

- this layer is not abandoned
- it is still a live part of the Saturn stack

## Relationship To The FPGA / Protocol 2 Handoff

The FPGA / Protocol 2 handoff explains what the hardware and wire protocol are
capable of.

This LinuxDriver / XDMA handoff explains how Linux reaches that hardware.

The relationship is:

- FPGA defines the endpoint and AXI/stream/register architecture
- XDMA exposes that endpoint to Linux as `/dev/xdma*`
- `P2_app` / `P3_app` sit on top of those device nodes and implement the client
  protocol

So the stack is:

1. FPGA
2. PCIe enumeration
3. XDMA kernel module
4. `/dev/xdma*`
5. Saturn app layer
6. Protocol 2 clients

This is the correct order for debugging.

## Recommended Field Debug Order

When a user reports "XDMA" or "p2app" startup failure, the fastest structured
debug order is:

1. Check PCIe endpoint presence:
   - `journalctl -k -b | grep -iE 'pcie|brcm-pcie|xilinx|xdma'`
   - `lspci -nn`
2. Check module state:
   - `lsmod | grep xdma`
   - `modinfo xdma`
3. Check device nodes:
   - `ls -l /dev/xdma0_user /dev/xdma/card0 2>/dev/null`
4. If endpoint exists but nodes do not:
   - run `fix-xdma.sh`
5. Only after nodes exist:
   - debug `p2app.service` / `p3app.service`

This order avoids mislabeling enumeration failures as compile failures.

## Recommended Robustness Plan

To make PCIe / XDMA bring-up more deterministic across reboot and kernel
updates, the next implementation should treat the stack explicitly as:

1. FPGA endpoint present on PCIe
2. `xdma` module installed for the running kernel
3. `xdma` module loaded and bound
4. `/dev/xdma*` nodes present
5. Saturn app allowed to start

Current implementation status:

- `scripts/saturn-xdma-doctor.sh` now exists as the first-line diagnostic helper
- `scripts/saturn-xdma-ready.sh` now exists as the dedicated readiness helper
- `scripts/fix-xdma.sh` now ends with doctor output and calls the doctor on
  modprobe/start failures
- `sw_tools/p2app-control/install.sh` now installs `saturn-xdma-ready.service`
  and makes `p2app.service` depend on it

The recommended rollout is:

### 1. Add A Single Doctor Script

Add a read-mostly diagnostic helper, for example:

- `scripts/saturn-xdma-doctor.sh`

It should report a single failure stage such as:

- `PCIE_ENDPOINT_MISSING`
- `XDMA_MODULE_NOT_INSTALLED_FOR_KERNEL`
- `XDMA_MODPROBE_FAILED`
- `XDMA_NOT_BOUND`
- `XDMA_DEVNODE_MISSING`
- `XDMA_UDEV_SYMLINK_MISSING`
- `P2APP_START_FAILED`
- `OK`

And it should collect the minimum evidence needed to classify the failure:

- running kernel version
- PCIe endpoint presence (`10ee:7024`)
- `xdma` module state
- binding state
- `/dev/xdma0_user` and `/dev/xdma/card0/*`
- recent kernel log lines for PCIe / XDMA
- `p2app.service` status

This should become the supported first-line diagnostic path for field support.

### 2. Add A Dedicated XDMA Readiness Service

Instead of having `p2app.service` own XDMA readiness through an `ExecStartPre`
poll loop, add a dedicated systemd unit whose only job is to confirm:

- the Xilinx endpoint is present
- `xdma` can be loaded
- udev has settled
- `/dev/xdma0_user` exists

Then make `p2app.service` depend on that readiness unit.

This keeps "hardware / driver ready" separate from "radio app started."

### 3. Keep `fix-xdma.sh`, But Make It The Supported Repair Path

`fix-xdma.sh` should remain the supported repair action, but it should end by
calling the doctor helper so that failures are classified correctly.

That is especially important for cases where:

- the endpoint never enumerated
- the module rebuild succeeded but binding still failed
- udev links are missing even though the module loaded

In other words, `fix-xdma.sh` should repair and then explain the remaining
failure stage.

### 4. Add Kernel Post-Install Rebuild Staging

Near term, a lightweight `/etc/kernel/postinst.d/` hook is the least risky
improvement.

That hook should rebuild or pre-stage `xdma.ko` for newly installed kernels
using the same logic already present in `fix-xdma.sh`.

This is a smaller step than full DKMS conversion, but it addresses the most
common "kernel updated, XDMA vanished after reboot" class of failure.

### 5. Surface The Result In Saturn Go

Once the doctor helper exists, the Update Manager should expose it directly as
an `XDMA Doctor` action so support can ask users for one structured report
instead of an ad hoc list of commands.

## Working Conclusions To Preserve

- Saturn uses a patched out-of-tree Xilinx XDMA driver.
- The supported path is `fix-xdma.sh`, not manual ad hoc rebuilding.
- This layer is kernel-version-sensitive because it is not DKMS-managed.
- The Saturn FPGA endpoint device ID `10ee:7024` is already supported.
- Missing `/dev/xdma0_user` can mean either:
  - endpoint not present on PCIe
  - endpoint present but driver/module path failed
- A `link down` or missing `lspci` endpoint is upstream of the Linux driver.
- The Linux driver is the bridge between FPGA capability and `P2_app` /
  `P3_app`, not the place where Protocol 2 semantics are defined.

## Recommended Next Step

The next useful companion note after this one would be:

- an address-map walkthrough that ties:
  - FPGA AXI-Lite regions
  - `saturnregisters.c`
  - XDMA user/control/register access
  - `P2_app` / `P3_app` register usage

That would complete the end-to-end handoff from FPGA to Linux driver to app.
