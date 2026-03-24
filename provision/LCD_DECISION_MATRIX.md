# Saturn LCD Decision Matrix (CM4/CM5, 7"/8")

This document captures the current known-good overlay mapping for Saturn provisioning on Raspberry Pi OS Trixie.

## Summary

- 7" Waveshare DSI LCD (C), 800x480:
  - Use `dtoverlay=vc4-kms-dsi-waveshare-800x480`
- CM4 custom JD 7" profile:
  - Use profile id `cm4-7-custom-jd`
  - Render `dtoverlay=uart3`
  - Render `dtoverlay=vc4-kms-dsi-waveshare-800x480`
- Laurence-style 7" single-DSI profile:
  - Use `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0`
  - Pair with `dtoverlay=uart3` on CM4 or `dtoverlay=uart2-pi5` on CM5
- 7" Waveshare DSI LCD (C), Saturn G2 CM5 dual-DSI field configuration:
  - Use `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0,dsi1`
  - And `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c1,dsi0`
- 8" Waveshare DSI LCD:
  - Use `dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1`

UART overlay is selected by compute module:

- CM4: `dtoverlay=uart3`
- CM5: `dtoverlay=uart2-pi5`

## Matrix

| Platform | LCD size/family | UART overlay | Panel overlay |
|---|---|---|---|
| CM4 | 7" Waveshare DSI LCD (C), 800x480 | `dtoverlay=uart3` | `dtoverlay=vc4-kms-dsi-waveshare-800x480` |
| CM4 | 7" custom JD profile (`cm4-7-custom-jd`) | `dtoverlay=uart3` | `dtoverlay=vc4-kms-dsi-waveshare-800x480` |
| CM4 | 7" Laurence-style single-DSI (`cm4-7-g2-single-dsi`) | `dtoverlay=uart3` | `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0` |
| CM4 | 8" Waveshare DSI LCD | `dtoverlay=uart3` | `dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1` |
| CM5 | 7" Waveshare DSI LCD (C), 800x480 | `dtoverlay=uart2-pi5` | `dtoverlay=vc4-kms-dsi-waveshare-800x480` |
| CM5 | 7" Laurence-style single-DSI (`cm5-7-g2-single-dsi`) | `dtoverlay=uart2-pi5` | `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0` |
| CM5 Saturn G2 | 7" Waveshare DSI LCD (C), dual-DSI field profile | `dtoverlay=uart2-pi5` | `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0,dsi1` and `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c1,dsi0` |
| CM5 | 8" Waveshare DSI LCD | `dtoverlay=uart2-pi5` | `dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1` |

## Why 7" uses `vc4-kms-dsi-waveshare-800x480`

Field testing showed the `...-panel,7_0_inchC,...` path often reports DSI connected but leaves the display black with control-path I2C failures (for example `Goodix`/`ws_touchscreen` errors).  
The `vc4-kms-dsi-waveshare-800x480` overlay matches the known-good behavior for the 7" (C) 800x480 panel on Trixie.

## CM5 Saturn G2 exception

Field testing on March 7, 2026 reported a working 7" Saturn G2 CM5 configuration only after reboot, using both DSI paths at once. That profile should be treated as a board-specific exception rather than collapsed into the generic CM5 7" rule.

## Notes

- Keep only one Waveshare DSI panel overlay active in `config.txt`, except for the CM5 Saturn G2 dual-DSI field profile above.
- Saturn auto-detect now preserves explicit managed profile ids from the `# Saturn managed LCD profile: ...` comment when present in the managed block.
- Saturn also preserves Laurence-style single-DSI configs when it sees `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0` paired with the expected UART overlay.
- If detection is ambiguous, set `SATURN_LCD_SIZE_INCH=7` or `SATURN_LCD_SIZE_INCH=8`.
- If you override `SATURN_LCD_I2C_DETECT_ADDR`, keep it in the valid `i2cdetect` range (`0x08..0x77`).
- Auto-detection script:
  - `scripts/detect-lcd-profile.sh`
  - Supports `--json` and `--emit-config` for automation pipelines.
