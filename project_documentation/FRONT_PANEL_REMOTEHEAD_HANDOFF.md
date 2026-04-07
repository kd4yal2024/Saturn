# Front Panel / RemoteHead Handoff

Date: 2026-03-30

## Purpose

This internal handoff note captures the current engineering direction for:

- Saturn front-panel identification
- `RemoteHead` modeling
- serial alias creation
- safe use of `ZZZS;` probing
- LCD/profile implications for `CM4` / `CM5`

It is intended to preserve the outcome of the March 29-30, 2026 email thread
between Jerry, Laurence, Abhi, and Christoph so later provisioning work does
not bake in the wrong abstractions.

## Key Inputs From The Thread

### Abhi Prakash

- All shipped `7"` `G2` units used `CM4`.
- On `CM5` with `no display`, the current install path ended up selecting an
  `8"` display config.
- There may be a real `CM4` vs `CM5` difference in how the `7"` panel's DSI/I2C
  path is wired or enabled.

### Laurence Barker

- `7"` works on `CM4` under Trixie.
- `7"` does not yet work reliably on `CM5`.
- For `CM5` with `no display`, it is acceptable to use the `8"` display file.
- The `RemoteHead` concept should be a Pi5/CM5-class controller product with
  display/front panel and no local XDMA/FPGA.
- If possible, identifying `RemoteHead` from:
  - no XDMA present
  - `G2V2`-class panel present
  - safe targeted panel response
  would be easier than introducing more udev rule splits.

### Christoph v. Wuellen

- `RemoteHead` should be treated as a product role, not a front-panel hardware
  type.
- The primary distinction between a normal `G2` and a `RemoteHead` is the
  presence or absence of the local FPGA/XDMA path.
- It is not good practice for `pihpsdr` to open arbitrary serial ports and send
  `ZZZS;` broadly.
- Automatic serial alias creation is acceptable only if probing is bounded and
  known-safe.
- Users must still be able to specify a serial port explicitly.

## Current Direction

### 1. Separate Front-Panel Hardware From System Role

Do not collapse these into a single concept.

Use:

- `front_panel_hw = G2V1 | G2V2 | NONE`
- `system_role = local_saturn | remotehead`

Do not use:

- `front_panel_hw = G2V1 | G2V2 | RemoteHead`

`RemoteHead` is a deployment/product role, not a front-panel type.

### 2. RemoteHead Should Not Be Inferred From Panel Response Alone

`ZZZS08` is not currently strong enough to stand alone as the authoritative
`RemoteHead` identifier.

The better model is:

- panel identity from a safe targeted probe
- plus `no XDMA/FPGA`
- plus broader platform/product context

### 3. Support Christoph's Safety Constraint

The system should move toward:

- safe identification
- bounded serial probing
- udev-created stable aliases
- manual override when needed

The system should move away from:

- broad scanning of arbitrary `/dev/tty*`
- using `ZZZS;` as a general serial discovery protocol
- pushing aggressive probing logic into `pihpsdr`

## Recommended Serial-Probing Policy

### Acceptable

- Probe only a small allowlist of candidate front-panel ports.
- Prefer known onboard UART paths or a very small set of expected serial
  candidates.
- Use `ZZZS;` as a targeted confirmation step, not as a blind discovery
  mechanism.
- Persist the result into provisioning state and/or stable udev aliases.
- Allow explicit manual port selection in applications.

### Not Acceptable

- Sweeping all `/dev/tty*` devices and sending `ZZZS;`
- Assuming any `ZZZS08` reply means `RemoteHead`
- Making application startup depend on unbounded serial auto-discovery

## LCD / Platform Policy From The Current Thread

### Stable Assumptions

- All shipped `7"` `G2` units were `CM4`.
- `CM5 + no display` may safely use the `8"` display file / config path.

### Conservative Auto-Selection Policy

- `CM4 + 7" G2` -> use the validated single-DSI `G2` path
- `CM5 + no display` -> use the `8"` / no-display-safe path
- `CM5 + 7"` -> manual only until the `I2C0` / overlay issue is understood

### Important Implication

The earlier field workaround where a `CM5` `G2V1` `7"` unit could be recovered
by selecting a dual-DSI profile should be treated as:

- useful recovery data

not:

- a factory-safe default rule

## Design Direction For Saturn

Provisioning and helper scripts may do limited, hardware-aware identification.
Runtime applications should consume the result, not rediscover it broadly.

Preferred stack:

1. Provisioning does bounded front-panel identification.
2. Provisioning records:
   - `front_panel_hw`
   - `system_role` when confidence is sufficient
3. udev provides stable aliases when appropriate.
4. `pihpsdr` and related apps use:
   - the stable alias, or
   - an explicit user-selected port

## Concrete Engineering Guidance

### Keep

- targeted pre-udev probing of a bounded candidate list
- post-udev use of stable aliases
- explicit user override paths
- `RemoteHead` modeled separately from front-panel hardware

### Avoid

- generic serial sweeps
- treating `ZZZS08` as a final `RemoteHead` verdict
- automatic `CM5 + 7"` LCD guesses

## DIY / Experimenter Reality

Ham-radio users often do more than operate a fixed factory configuration.
They may:

- swap compute modules
- reuse partial wiring from another build
- mix displays, panels, and carrier boards
- attach extra USB serial devices
- test combinations that work as a one-off but are not validated production
  baselines

This increases the risk that a successful field workaround gets mistaken for a
safe default provisioning rule.

Because of that, Saturn should clearly separate:

- validated factory/default behavior
- manual recovery options
- experimental combinations

The practical policy should be:

- keep automatic behavior narrow, conservative, and well-bounded
- only auto-apply rules for combinations that are validated and repeatable
- preserve experimental or one-off field successes as manual helper-tool
  options, not provisioning defaults
- keep explicit user override first-class for builders and experimenters
- warn clearly when a detected combination is unsupported or provisional

## Open Questions

- What exact raw serial candidate paths are considered safe to probe before
  udev renaming?
- Is `no XDMA` alone sufficient for `RemoteHead`, or does the product need
  another confirming signal?
- What is the exact `CM5` `7"` `I2C0` / DSI enablement problem versus `CM4`?
- Should Saturn add an explicit `cm5-headless` profile name, or continue to
  reuse the `8"` / no-display-safe file internally?

## Bottom Line

To support Christoph's recommendation, the project direction should be:

- conservative serial auto-detection
- strong separation between hardware identity and system role
- udev-based stable naming where possible
- manual fallback when confidence is low

That is the safest path for Saturn provisioning and for any future integration
with `pihpsdr`.

## Current Repo Status (April 6, 2026)

The current Saturn checkout already has some of the right building blocks:

- first-boot provisioning is centralized in `provision/cloud-init/provision-saturn.sh`
- provisioning already does bounded front-panel detection before and after udev
- LCD/profile logic is already centralized in `scripts/saturn-lcd-lib.sh`
- `CM5 + 7"` is already treated conservatively and requires explicit/manual
  profile selection
- `rules/install-rules.sh` already supports a standard rules file plus an
  explicit override path
- the update manager is already acting as a maintenance plane rather than a
  first-boot hardware configurator

The main gaps are:

- provisioning now records a conservative `system_role`, but only for reporting
  and support visibility
- `RemoteHead` is documented as a separate role, but role-specific boot
  behavior is still intentionally not auto-applied
- no validated automatic role-specific boot behavior should be applied yet for
  `RemoteHead`

## Recommended Near-Term Path

The safest incremental path from the current repo state is:

1. Keep Raspberry Pi image selection coarse: `CM4` image vs `CM5` image.
2. Keep update-manager focused on software maintenance and diagnostics, not on
   rewriting boot-hardware policy during normal updates.
3. Extend provisioning state conservatively so it records both:
   - `front_panel_hw`
   - `system_role`
4. Continue to treat:
   - `CM5 + 7"` as manual/explicit-only
   - `RemoteHead` as conservative/manual-override territory until validated
5. Only add automatic role-specific behavior after the detection criteria are
   stable and repeatable on real hardware.

## Provisioning Profile File Direction (April 7, 2026)

Laurence suggested that provisioning should emit a simple summary file so later
software, including `piHPSDR`, can consume the discovered hardware facts
without inheriting G2-only probing assumptions.

That direction matches the current Saturn architecture and should build on the
state provisioning already writes under `/var/lib/saturn-provision/` rather
than introducing a parallel user-home mechanism such as `~/.G2/profile`.

### Current Facts Already Available

Provisioning already records:

- `complete`
  - `hardware_model`
  - `hardware_platform_vendor`
  - `hardware_module_family`
  - `hardware_storage_variant`
  - `xdma_present`
  - `system_role`
- `front-panel-type`
- `system-role`

### Recommended Next Step

Add one canonical machine-readable provisioning profile file:

- `/var/lib/saturn-provision/profile.env`
- optionally later `/var/lib/saturn-provision/profile.json`

Keep the fields factual, not policy-heavy. Good first fields are:

- `radio_profile`
- `discovered_processor`
- `hardware_model`
- `hardware_platform_vendor`
- `hardware_module_family`
- `hardware_storage_variant`
- `front_panel_type`
- `front_panel_device_path`
- `xdma_present`
- `system_role`
- `expected_display_type`
- `lcd_profile`
- `display_size_inch`
- `uart_overlay`
- `ganymede_present`
- `ganymede_device_path`
- `aries_present`
- `aries_device_path`

### Important Boundary

This file should be treated as:

- provisioning output
- a stable fact source for support and applications

not:

- a mandate that `piHPSDR` become G2-specific
- a replacement for explicit user override paths

`piHPSDR` should be able to consume this file opportunistically when present,
while continuing to function normally on other platforms when it is absent.

It should not replace the current discovery code yet.

The practical near-term model is:

- keep the existing discovery paths, udev aliases, and bounded probes
- add the provisioning profile file as an extra fact source for future upgrade
  and support workflows
- allow later software to use it opportunistically when available, while still
  working correctly when it is absent on older systems

### Enough To Start?

Yes.

The project already has enough to begin because:

- provisioning is already centralized
- front-panel detection is already bounded
- `system_role` is already recorded conservatively
- hardware classification already exists

The next implementation should therefore be narrow:

1. write `profile.env`
2. keep it factual and support-oriented
3. do not change runtime behavior yet
4. let later application work decide which fields to consume
