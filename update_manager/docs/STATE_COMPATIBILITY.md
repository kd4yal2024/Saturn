# Saturn Persistent-State Compatibility

REM-0205 defines how an immutable application release may consume or migrate
mutable operator state without silently making the prior release unusable.
This is an application-release contract; it does not replace the broader
REM-0301 appliance inventory in `STATE_INVENTORY.md` or the separate backup
formats planned by REM-0302.

## State versions

The absence of `/var/lib/saturn-state/state-schema.json` means legacy state
schema 0. New release-manifest schema 2 declares state schema 1. Its initial
migration writes only the version marker; the existing managed file formats
do not change. Installed release-manifest schema 1 bundles are treated as
legacy state-schema-0 releases and are declared able to read schemas 0 and 1.

A release contract must declare its own state version, the versions it reads,
the immediately preceding migration input, migration documentation, and the
direct files it manages. The manifest validator rejects a release that cannot
read both its own state and the immediately preceding schema.

## Managed state

The schema-1 contract covers these operator/application files when present:

```text
custom_scripts.json
remote_profiles.json
remote_settings.json
repo_root.txt
saturngo_update_policy.json
update_policy.json
state-schema.json
```

Logs, build caches, deployment history, administrator credentials, TLS keys,
remembered-device material, Tailscale identity, boot configuration, and FPGA
artifacts are not part of this migration snapshot. `STATE_INVENTORY.md`
classifies them; REM-0302 will define the actual backup formats.

## Activation sequence

Before the release pointer changes, the root activator:

1. validates the installed release and state contract;
2. reads the current marker and preflights target and rollback compatibility;
3. persists the deployment intent;
4. stops Saturn Go, Saturn Bridge, and P2 so managed state is quiescent;
5. creates and flushes a root-owned backup with per-file metadata and SHA-256;
6. performs the declared migration and writes the marker atomically;
7. wires and atomically selects the target release; and
8. starts services and requires exact-target readiness before commit.

If migration fails, the pointer is never changed and the prior services are
restored. If any later activation step fails, the backup is validated and
restored before the prior services restart. The transaction records the state
plan, backup directory, migration result, and any explicit one-way approval.

## One-way migrations

A deployment is rollback-unsafe when the prior release does not declare that
it can read the target state version. Such a migration is blocked unless the
target contract explicitly marks and documents it as one-way and a local root
operator supplies `--approve-one-way-migration`. Saturn Go receives no sudoers
permission for that helper or flag.

One-way approval is an exceptional release decision, not a general override.
The release documentation must describe which formats change and the manual
recovery path. Normal releases should remain backward-readable and use the
automatic rollback path.

## Current production status

The helper and isolated failure-injection tests are implemented, but release
activation remains disabled by root-owned policy. REM-0205 implementation and
tests do not create `/opt/saturn/current`, restart services, or activate a live
release. A separate, explicitly approved appliance rollback test is still
required before production activation is enabled.
