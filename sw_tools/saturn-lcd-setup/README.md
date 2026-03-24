# Saturn LCD Setup

`saturn-lcd-setup` is a standalone GTK desktop tool for selecting, previewing, applying, and restoring Saturn LCD boot profiles on Raspberry Pi desktop images.

## What It Does

- inspects the current LCD-related boot configuration
- shows the currently detected compute module, LCD profile, front-panel type, and active Waveshare overlay lines
- previews the managed LCD block before any change is written
- applies a selected LCD profile through a privileged helper
- creates a timestamped backup of `config.txt` before each apply
- restores any previously saved LCD backup from the UI
- prompts for reboot after apply or restore

## Profiles

Current supported profiles:

- `auto`
- `cm4-7`
- `cm4-7-custom-jd`
- `cm4-7-g2-single-dsi`
- `cm4-8`
- `cm5-7`
- `cm5-7-g2-single-dsi`
- `cm5-7-g2-dual-dsi`
- `cm5-8`

The profile logic is shared with Saturn provisioning by reusing helpers from:

- `provision/cloud-init/provision-saturn.sh`
- `scripts/detect-front-panel.sh`
- `scripts/detect-lcd-profile.sh`
- `scripts/saturn-lcd-helper.sh`

## Build

From the repo root:

```bash
make -C sw_tools/saturn-lcd-setup
```

This produces:

- `sw_tools/saturn-lcd-setup/saturn-lcd-setup`

## Desktop Integration

The launcher is installed by:

- `scripts/update-desktop-apps.sh`

Launcher name:

- `SaturnLCDSetup.desktop`

Provisioning also builds this tool during the standard app/tool build stage.

## Apply And Restore Behavior

Apply flow:

1. preview selected profile
2. request confirmation
3. create a backup of `config.txt`
4. apply the managed LCD block
5. prompt for reboot

Backups are stored next to `config.txt` as:

- `config.txt.bak.lcd-tool.<timestamp>`

Restore flow:

- select a backup from the dropdown
- restore it through the privileged helper
- reboot afterward

## Helper Commands

The UI uses:

- `scripts/saturn-lcd-helper.sh detect`
- `scripts/saturn-lcd-helper.sh profiles`
- `scripts/saturn-lcd-helper.sh preview --profile <profile>`
- `scripts/saturn-lcd-helper.sh apply --profile <profile>`
- `scripts/saturn-lcd-helper.sh backups`
- `scripts/saturn-lcd-helper.sh restore --backup <path>`

## Notes

- LCD changes are boot-config changes and generally require reboot
- the tool is intentionally conservative: it previews before apply and always creates a backup
- the GTK app itself runs as the desktop user; privileged actions are performed with `pkexec`
- the detected-state summary also includes front-panel type from `scripts/detect-front-panel.sh` when available
