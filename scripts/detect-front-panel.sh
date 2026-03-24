#!/usr/bin/env bash
set -euo pipefail

# Detect Saturn front-panel hardware and print exactly one of:
#   G2V1
#   G2V2
#   NONE

PANEL_SERIAL_PATH="${SATURN_PANEL_SERIAL_PATH:-/dev/serial/by-id/g2-front-9600}"
PANEL_I2C_BUS="${SATURN_PANEL_I2C_BUS:-1}"
PANEL_I2C_ADDR="${SATURN_PANEL_I2C_ADDR:-0x20}"

if ! command -v python3 >/dev/null 2>&1; then
  printf 'NONE\n'
  exit 0
fi

python3 - "$PANEL_SERIAL_PATH" "$PANEL_I2C_BUS" "$PANEL_I2C_ADDR" <<'PY'
import os
import sys

serial_path = sys.argv[1]
i2c_bus = int(sys.argv[2], 0)
i2c_addr = int(sys.argv[3], 0)

g2v2_found = False
g2v1_found = False

try:
    import serial  # type: ignore
except Exception:
    serial = None

try:
    from smbus import SMBus  # type: ignore
except Exception:
    try:
        from smbus2 import SMBus  # type: ignore
    except Exception:
        SMBus = None

if serial is not None and os.path.exists(serial_path):
    ser = None
    try:
        ser = serial.Serial(serial_path, 9600, timeout=1)
        try:
            ser.reset_input_buffer()
        except Exception:
            pass
        ser.write(b"ZZZS;")
        ser.flush()
        response = ser.read_until(b";")
        if response.startswith(b"ZZZS05"):
            g2v2_found = True
    except Exception:
        pass
    finally:
        if ser is not None:
            try:
                ser.close()
            except Exception:
                pass

if SMBus is not None:
    bus = None
    try:
        bus = SMBus(i2c_bus)
        bus.read_byte(i2c_addr)
        g2v1_found = True
    except Exception:
        pass
    finally:
        if bus is not None:
            try:
                bus.close()
            except Exception:
                pass

if g2v1_found:
    print("G2V1")
elif g2v2_found:
    print("G2V2")
else:
    print("NONE")
PY
