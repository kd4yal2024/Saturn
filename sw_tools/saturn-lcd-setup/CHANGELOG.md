# Changelog

## 2026-03-25

- LCD/profile resolution now comes from shared library `scripts/saturn-lcd-lib.sh`
- the helper now prefers the saved provisioning front-panel state file over a live re-probe
- `scripts/saturn-lcd-helper.sh detect` now reports both `front_panel_type=` and `front_panel_source=`
- helper auto/apply paths now use the same resolved profile that is reported to the user
- front-panel-aware G2 7-inch auto-selection is now available on first provisioning, not only in helper/reprovision flows
- front-panel detection now accepts `RemoteHead` in both saved-state and live-probe paths; the LCD G2 tiebreaker remains limited to `G2V1` and `G2V2`

## 2026-03-23

- added `cm4-7-custom-jd` LCD profile to the shared helper output
- LCD auto-detect now preserves Saturn-managed explicit profile ids from the managed block comment
- added `cm4-7-g2-single-dsi` and `cm5-7-g2-single-dsi` LCD profiles to the shared helper output
- LCD auto-detect now preserves Laurence-style single-DSI 7-inch `config.txt` overlays
- surfaced front-panel detection in `saturn-lcd-setup`
- `scripts/saturn-lcd-helper.sh detect` now reports `front_panel_type=G2V1|G2V2|RemoteHead|NONE|unknown`

## 2026-03-08

- added first standalone `saturn-lcd-setup` GTK desktop application
- added shared helper script `scripts/saturn-lcd-helper.sh`
- added profile preview before apply
- added timestamped `config.txt` backup creation before LCD changes
- added UI restore support for LCD config backups
- added backup dropdown selection in the UI for multiple restore points
- added reboot actions after apply/restore
- integrated tool build into provisioning
- integrated desktop launcher generation through `scripts/update-desktop-apps.sh`
