# Rust Supply Chain Policy

Saturn ships two Rust binaries:

- `update_manager/rust-server` (`saturn-go`)
- `update_manager/saturn-bridge`

Both crates keep their own `Cargo.lock`. Release, CI, and field-update builds
must use the lockfile with `--locked`; an unexpected lockfile edit is treated as
a dependency change.

## Required Checks

Run these for each Rust crate:

```bash
cargo fmt -- --check
cargo audit --deny warnings
cargo deny check --config ../deny.toml advisories bans sources
cargo test --locked
```

The shared deny policy lives at `update_manager/deny.toml`.

## Dependency Policy

- Prefer crates.io dependencies only.
- Do not use wildcard dependency requirements.
- Keep `Cargo.lock` committed for both binary crates.
- Review dependency changes as code changes, not formatting churn.
- Use explicit pins only when runtime behavior depends on a known-compatible
  version. Example: `saturn-bridge` currently pins `tungstenite` because it is
  part of the live TCI/WebSocket transport path.
- Revisit pinned transport/security-sensitive crates during beta release
  reviews and after any `cargo audit` advisory.
- `cargo deny` license checks are not enabled yet. License review remains a
  manual dependency-review item for beta changes.

## Updating Dependencies

Update one dependency family at a time when possible:

```bash
cd update_manager/rust-server
cargo update -p crate-name
cargo test --locked

cd ../saturn-bridge
cargo update -p crate-name
cargo test --locked
```

After dependency updates, rerun the audit and deny checks above and document any
intentional advisory ignore in `update_manager/deny.toml`.
