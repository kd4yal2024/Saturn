# PROM/BIN export record

The Saturn complete configuration image is a 32-Mbit, uncompressed `SPIx1`
BIN image. The export also creates a slot-relative primary BIN specifically
for the CM4 `load-FPGA` utility.
The multiboot layout is fixed and must not be changed without updating the CM4
loader and fallback recovery procedure:

| Address | Payload |
| --- | --- |
| `0x00000000` | golden/fallback bitstream |
| `0x0097FC00` | `timer1.bin` |
| `0x00980000` | primary development bitstream |
| `0x01300000` | `timer2.bin` |

The authoritative existing export is `FPGA/multiboot_address_table/saturngolden.bin`
and its Vivado report is `saturngolden.prm`. It is byte-for-byte identical to
`FPGA/saturnfallback.bin` (SHA256
`543b207750e0aafe1c63ffb377efa4fea4624527a5f6c09744291779b096f037`). The
historical primary payload `FPGA/saturnprimary2024V27.bin` is 9,730,652 bytes
(SHA256
`e159b1167603405b358a534b20319decd72716849eea4c44d9abe84aad0a8218`).

## Reproducible Vivado export

After a successful bitstream build, run this from a Vivado 2023.1 Tcl console
or a Windows Vivado command prompt:

```text
vivado -mode batch -nolog -nojournal -source FPGA/lab/tcl/export-prom.tcl
```

The script defaults to the known golden bitstream, the current-HEAD lab
bitstream (`results/vivado/saturn-v30-<git-sha>.bit`), and the checked-in timer
payloads. It creates two deliberately distinct artifacts:

- `saturn-primary-v30-<git-sha>.bin`: slot-relative primary payload; this is the
  only generated artifact suitable for `load-FPGA -b ... -v` without `-f`.
- `saturn-lab.bin`: complete address-zero multiboot image for archival or an
  external programmer; never pass this file to the default `load-FPGA` path.

To
export a different primary bitstream, set `SATURN_PRIMARY_BIT` to its path and
`SATURN_PROM_OUTPUT`/`SATURN_PRIMARY_BIN` to the desired output paths. Vivado
writes both BIN files and adjacent PRM reports. The script verifies the files,
checks that the primary payload ends below the `0x01300000` timer barrier, and
prints `SATURN_LAB_PROM_OK`. It also writes `prom-manifest.json` with source
identity, hashes, primary byte count, and exact loader erase interval.

The equivalent command, matching the original project instructions, is:

```tcl
write_cfgmem -format bin -size 32 -interface SPIx1 \
  -loadbit "up 0x00000000 saturn_top_wrapper_golden.bit up 0x00980000 saturn_top_wrapper.bit" \
  -loaddata "up 0x0097FC00 timer1.bin up 0x01300000 timer2.bin" \
  saturngolden.bin -force
```

This record is export-only. CM4/XDMA programming and G2/RF validation require
an operator-approved hardware step. Normal development must never use
`load-FPGA -f`, and must never give `saturn-lab.bin` to `load-FPGA`.
