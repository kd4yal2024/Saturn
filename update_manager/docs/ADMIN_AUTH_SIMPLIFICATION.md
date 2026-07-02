# Admin Auth Simplification Plan

Saturn's users are amateur radio operators, often 60+, running the appliance on
a home LAN and sometimes over Tailscale. Password *maintenance* — not password
*strength* — is the dominant real-world failure mode. This plan makes auth
simple enough to survive that audience while staying sound for the one case
where strength matters (an operator who port-forwards the TLS listener to the
open internet).

Guiding rules (aligned with NIST 800-63B):

- Length over complexity. Minimum 5 characters; **no** composition rules, no
  forced rotation, no lockout spirals.
- One password, one source of truth. The user should never learn that two
  auth backends exist.
- Physical access is the trust anchor. Recovery happens at the console or the
  front panel, never through an email/reset flow.
- Prefer making the password *rarely needed* over making it strong: trusted
  transports (Tailscale) and long-lived device cookies beat complexity rules.

## Threat model

| Network path | Realistic attacker | Password's job |
|---|---|---|
| Home LAN (nginx :80, `/saturn/*`) | guests on the wifi | keep casual hands off TX and admin actions |
| Tailscale (TLS :8443 via Serve) | nobody — tailnet is already authenticated | redundant; Phase 3 removes it here |
| Port-forwarded WAN (TLS :8443) | internet scanners | real defense; generated passphrases + failure delay |

## Phase 1 — one password everywhere (implemented)

- `scripts/saturn-admin-password.sh` (installed to the privileged helpers dir):
  - `set` — password on stdin; updates `/etc/nginx/.htpasswd` (effective
    immediately; nginx reads it per-request) **and** the
    `10-remote-auth.conf` drop-in (systemd-escaped), then schedules a
    deferred `saturn-go` restart (~2s) so the calling HTTP handler can finish
    its response before the TLS listener restarts.
  - `reset` — console recovery: generates a `word-word-word-NN` passphrase,
    applies it via `set`, prints it. Root/console only; not in sudoers.
  - `status` — reports backend presence and `sync_state` by verifying the
    drop-in plaintext against the htpasswd hash (`htpasswd -vi`; no
    password ever appears in argv).
- `set` is all-or-nothing: both backends are snapshotted first and restored
  together if the htpasswd write, drop-in write, or systemd reload fails, so
  a partial failure can never leave LAN and TLS holding different passwords.
- `/change_password` (rust-server `auth.rs`) now calls the helper via
  `sudo -n`; sudoers grants exactly `set` and `status` (no wildcard).
- Saturn Go UI: single password field, visible by default with a hide
  toggle, "Generate for me" passphrase button, no confirm-twice field.
- Generated passphrases use 4 words + 2 digits from CSPRNG sources
  (`/dev/urandom` in the helper, `crypto.getRandomValues()` in the UI) —
  strong enough for online guessing against the TLS listener, and Phase 2's
  failure delay raises that bar further.

Known duplication (accepted for Phase 1): `install_saturn_go_nginx.sh` still
carries its own htpasswd/drop-in write logic for first provisioning, since it
runs before the helper is installed. Follow-up: converge the installer on the
repo copy of `saturn-admin-password.sh` so there is exactly one writer.

## Phase 2 — make password entry rare (planned)

- "Remember this device" cookie on the TLS listener (~365 days, secure,
  HttpOnly) so a password is typed roughly once per browser.
- Small per-IP delay on repeated basic-auth failures on :8443 (invisible to
  legitimate users; blunts scanners in the port-forward case).

## Phase 3 — passwordless over Tailscale (planned)

- When a request provably arrives via local `tailscale serve` (loopback
  proxy + `Tailscale-User-Login` header), accept the tailnet identity and
  skip basic auth. The tailnet login is stronger auth than any password.
- Docs steer users to Tailscale instead of port forwarding: easier *and*
  safer.

## Explicitly rejected

- Passkeys/WebAuthn (recovery burden for this audience), TOTP/2FA, password
  expiry, complexity meters, shared default credentials (unattended installs
  keep generating device-unique random passwords).
