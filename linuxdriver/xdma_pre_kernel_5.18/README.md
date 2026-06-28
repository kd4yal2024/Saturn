# Retired XDMA Driver Tree

The pre-kernel-5.18 XDMA source tree is retired for Saturn beta releases.

Use the supported hardened driver in:

```text
linuxdriver/xdma
```

This directory remains only as a marker so old notes and links fail clearly
instead of silently building a stale driver. If an old-kernel recovery case
needs the previous code, recover it from git history and backport the active
tree's safety fixes before shipping or installing it.

