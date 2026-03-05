# Saturn LCD Decision Matrix (CM4/CM5, 7"/8")

This document captures the current known-good overlay mapping for Saturn provisioning on Raspberry Pi OS Trixie.

## Summary

- 7" Waveshare DSI LCD (C), 800x480:
  - Use `dtoverlay=vc4-kms-dsi-waveshare-800x480`
- 8" Waveshare DSI LCD:
  - Use `dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1`

UART overlay is selected by compute module:

- CM4: `dtoverlay=uart3`
- CM5: `dtoverlay=uart2-pi5`

## Matrix

| Platform | LCD size/family | UART overlay | Panel overlay |
|---|---|---|---|
| CM4 | 7" Waveshare DSI LCD (C), 800x480 | `dtoverlay=uart3` | `dtoverlay=vc4-kms-dsi-waveshare-800x480` |
| CM4 | 8" Waveshare DSI LCD | `dtoverlay=uart3` | `dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1` |
| CM5 | 7" Waveshare DSI LCD (C), 800x480 | `dtoverlay=uart2-pi5` | `dtoverlay=vc4-kms-dsi-waveshare-800x480` |
| CM5 | 8" Waveshare DSI LCD | `dtoverlay=uart2-pi5` | `dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1` |

## Why 7" uses `vc4-kms-dsi-waveshare-800x480`

Field testing showed the `...-panel,7_0_inchC,...` path often reports DSI connected but leaves the display black with control-path I2C failures (for example `Goodix`/`ws_touchscreen` errors).  
The `vc4-kms-dsi-waveshare-800x480` overlay matches the known-good behavior for the 7" (C) 800x480 panel on Trixie.

## Notes

- Keep only one Waveshare DSI panel overlay active in `config.txt`.
- If detection is ambiguous, set `SATURN_LCD_SIZE_INCH=7` or `SATURN_LCD_SIZE_INCH=8`.
- Auto-detection script:
  - `scripts/detect-lcd-profile.sh`
  - Supports `--json` and `--emit-config` for automation pipelines.
