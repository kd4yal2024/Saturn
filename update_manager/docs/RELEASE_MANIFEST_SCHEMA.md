# Saturn Application Release Manifest v1

Milestone 2 application releases are built as complete inactive directories.
The release builder does not install files, switch `/opt/saturn/current`, or
restart services. A separate root-owned installer copies a validated bundle
into its immutable versioned location. A later deployment transaction will
activate an installed directory.

## Bundle layout

```text
<release-root>/
  release-manifest.json
  SHA256SUMS
  bin/
    saturn-go
    saturn-bridge
    p2app
    ... normal-release radio tools ...
  webroot/
    ... Saturn Go and Saturn Remote assets ...
  scripts/
    ... versioned maintenance scripts ...
  share/release/components-v1.json
```

FPGA images, FPGA flashing tools, the XDMA kernel module, kernel/header files,
boot configuration, and whole-disk imaging tools are not part of a normal
application release.

## Manifest fields

`release-manifest.json` is UTF-8 JSON with these top-level fields:

| Field | Meaning |
|---|---|
| `format` | Literal `saturn-application-release`. |
| `schema_version` | Integer `1`. Unknown versions must fail closed. |
| `source` | Full lowercase Git commit, repository identity, and `dirty:false`. |
| `build` | UTC build time, architecture, operating system, and passed build/test gates. |
| `components` | Required named runtime components with role, version, source commit, path, mode, size, and SHA-256. |
| `files` | Exact inventory of every payload file except the manifest and checksum index. |

Every component maps to the same `source.commit`. Package-based components
also carry their Cargo or npm package version; native components use the full
source commit as their version.

`SHA256SUMS` covers every payload file plus `release-manifest.json`, and excludes
only itself. Validation rejects missing or extra files, path traversal,
symlinks, non-regular files, mode changes, size changes, digest changes,
incomplete build results, and a component set that differs from
`components-v1.json`. The validator also requires the exact build/test gate set
declared by that component policy; a caller cannot substitute one trivial
passing check for the normal release gates.

## Build contract

Run:

```bash
update_manager/scripts/saturn-release-build.sh
```

The source checkout must be clean and resolve to one full commit. On the
supported low-memory Pi, the builder requires the configured disk-backed build
swap and uses one Rust build job by default. It runs Rust server tests, Bridge
tests with native DSP stubs, Remote web type/seam/unit/build checks, Protocol 2
boundary tests, and release builds before creating the manifest.

The default output is:

```text
/var/lib/saturn-state/release-staging/<full-commit>/
```

A failure removes only the temporary build directory. It never changes the
active release pointer.

## Inactive installation contract

The canonical installer provisions trusted copies of the manifest validator,
component policy, and `saturn-release-install-root.sh` below
`/usr/local/lib/saturn-go`. To install a completed bundle without activating
it, run the root-owned helper with the bundle directory:

```bash
sudo /usr/local/lib/saturn-go/scripts/saturn-release-install-root.sh \
  /var/lib/saturn-state/release-staging/<full-commit>
```

The bundle must be a direct child of the configured staging root, be owned by
the configured build user, match its full-commit directory name and host
architecture, contain only regular non-writable files/directories, and pass
the trusted manifest/component-policy validator. The helper copies into a
private sibling directory, changes production ownership to `root:root`,
revalidates the installed copy, flushes it, and then renames it to:

```text
/opt/saturn/releases/<full-commit>/
```

An existing valid identical release is accepted idempotently. An invalid
existing destination fails closed. This operation never creates or changes
`/opt/saturn/current` and never restarts a service.
