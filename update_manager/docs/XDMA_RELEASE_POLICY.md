# XDMA Release Policy

Saturn has two XDMA maintenance paths:

- direct recovery rebuild: `scripts/fix-xdma.sh`
- DKMS-managed beta path: `scripts/install-xdma-dkms.sh`

The direct helper remains the fastest field recovery tool. The DKMS path is the
preferred beta path for systems that should rebuild XDMA automatically when a
new kernel package is installed.

## DKMS Package Version

The default DKMS package identity is:

```text
saturn-xdma/2020.1.8-saturn
```

Treat `SATURN_XDMA_DKMS_VERSION` as a release identifier. If the XDMA source or
DKMS template changes after a package version has been installed, either:

- bump `SATURN_XDMA_DKMS_VERSION`, or
- rerun `scripts/install-xdma-dkms.sh --force`.

The installer refuses to reuse an already registered package/version without
`--force` so stale source cannot silently remain active.

## Manual Hook Takeover

A successful DKMS install disables the legacy manual kernel postinst hook:

```text
/etc/kernel/postinst.d/saturn-xdma
```

The disabled hook is moved to:

```text
/etc/kernel/postinst.d/saturn-xdma.disabled-by-dkms
```

This prevents both DKMS and `fix-xdma.sh` from rebuilding/installing XDMA during
the same kernel package update.

Use `--keep-manual-postinst` only for diagnostics where both paths must remain
visible. Do not use it for normal beta images.

## Rollback

To remove the DKMS package and staged source:

```bash
sudo bash scripts/install-xdma-dkms.sh --uninstall
```

The uninstall path does not restore the manual postinst hook automatically. If a
system should go back to the direct helper path, reinstall or restore the hook
intentionally.
