# Admin Auth Simplification Plan

Saturn's users are amateur radio operators, often 60+, running the appliance on
a home LAN and sometimes over Tailscale. Password *maintenance* — not password
*strength* — is the dominant real-world failure mode. This plan makes auth
simple enough to survive that audience while staying sound for the one case
where strength matters (an operator who port-forwards the TLS listener to the
open internet).

Guiding rules for the Saturn appliance audience:

- At least 5 characters; **no** composition rules, forced rotation, or lockout
  spirals. Generated/recovery passwords remain five characters for ease of
  initial entry, while operators may choose longer values.
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
| Port-forwarded WAN (TLS :8443) | internet scanners | operator password plus failure delay; direct port forwarding is discouraged |

## Phase 1 — one password everywhere (implemented)

- `scripts/saturn-admin-password.sh` (installed to the privileged helpers dir):
  - `set` — password on stdin; updates `/etc/nginx/.htpasswd` (effective
    immediately; nginx reads it per-request) **and** the
    `10-remote-auth.conf` drop-in (systemd-escaped), then schedules a
    deferred `saturn-go` restart (~2s) so the calling HTTP handler can finish
    its response before the TLS listener restarts.
  - `reset` — console recovery: generates a five-character readable password,
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
  toggle, "Generate for me" five-character button, no confirm-twice field.
- Generated passwords use five lowercase letters/digits from a reduced,
  non-ambiguous alphabet and CSPRNG sources (`/dev/urandom` in the helper,
  `crypto.getRandomValues()` in the UI).

The full and Saturn-Go-only installers both call the repository copy of
`saturn-admin-password.sh` for first provisioning. Password changes, recovery,
and installation therefore share one transactional writer for both backends.

## Phase 2 — make password entry rare (implemented)

- "Remember this device" cookie on the TLS listener (`remote_tls.rs`):
  `saturn_remote_auth`, Max-Age 365 days, Secure, HttpOnly, SameSite=Strict.
  The token is `HMAC-SHA256(secret, current credential)` where the secret is
  32 random bytes persisted at `$SATURN_STATE_DIR/remote-tls/cookie.secret`
  (0600), so remembered devices survive saturn-go restarts and reboots, but a
  password change (which restarts saturn-go with a new credential) signs out
  every remembered device at once. If the secret cannot be persisted, the
  token falls back to the old per-process random value (session-only) with a
  warning. Credential and cookie comparisons are constant-time.
- Per-IP tarpit on repeated basic-auth failures on :8443: only requests that
  *carried* an Authorization header and got a 401 count (a first visit
  answered with the challenge stays instant). The first 2 failures per IP are
  free — a human mistyping never notices — then delays grow 1s → 2s → 4s →
  8s, capped at 10s, and are forgotten after 15 minutes of quiet. State is
  in-memory and bounded (4096 IPs). Behind `tailscale serve` all requests
  share the loopback source IP; that's acceptable because tailnet users are
  already authenticated and Phase 3 removes the password there entirely.

## Phase 3 — passwordless over Tailscale (planned)

- When a request provably arrives via local `tailscale serve` (loopback
  proxy + `Tailscale-User-Login` header), accept the tailnet identity and
  skip basic auth. The tailnet login is stronger auth than any password.
- Docs steer users to Tailscale instead of port forwarding: easier *and*
  safer.

## Explicitly rejected

- Passkeys/WebAuthn (recovery burden for this audience), TOTP/2FA, password
  expiry, complexity meters, and shared default credentials (unattended
  installs generate a device-unique five-character password).
