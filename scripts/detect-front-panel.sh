#!/usr/bin/env bash
set -euo pipefail

# Detect Saturn front-panel hardware and print exactly one of:
#   G2V1
#   G2V2
#   NONE
#
# Modes:
#   --post-udev (default): prefer the stable aliased serial path
#   --pre-udev            : probe raw serial device nodes before udev renaming

PANEL_DETECT_MODE="${SATURN_PANEL_DETECT_MODE:-post-udev}"
PANEL_SERIAL_PATH="${SATURN_PANEL_SERIAL_PATH:-/dev/serial/by-id/g2-front-9600}"
PANEL_SERIAL_PATH_RAW="${SATURN_PANEL_SERIAL_PATH_RAW:-}"
PANEL_SERIAL_CANDIDATES_RAW="${SATURN_PANEL_SERIAL_CANDIDATES_RAW:-}"
PANEL_I2C_BUS="${SATURN_PANEL_I2C_BUS:-1}"
PANEL_I2C_ADDR="${SATURN_PANEL_I2C_ADDR:-0x20}"

usage() {
  cat <<'EOF'
Usage: detect-front-panel.sh [--pre-udev|--post-udev] [--serial-path PATH] [--raw-serial-path PATH]

Detect Saturn front-panel hardware and print exactly one of:
  G2V1
  G2V2
  NONE

Environment:
  SATURN_PANEL_DETECT_MODE=post-udev|pre-udev
  SATURN_PANEL_SERIAL_PATH=/dev/serial/by-id/g2-front-9600
  SATURN_PANEL_SERIAL_PATH_RAW=/dev/ttyAMA3
  SATURN_PANEL_SERIAL_CANDIDATES_RAW=/dev/ttyAMA3:/dev/ttyACM0
  SATURN_PANEL_I2C_BUS=1
  SATURN_PANEL_I2C_ADDR=0x20
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --pre-udev)
      PANEL_DETECT_MODE="pre-udev"
      shift
      ;;
    --post-udev)
      PANEL_DETECT_MODE="post-udev"
      shift
      ;;
    --serial-path)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      PANEL_SERIAL_PATH="$2"
      shift 2
      ;;
    --raw-serial-path)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      PANEL_SERIAL_PATH_RAW="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

if ! command -v python3 >/dev/null 2>&1; then
  printf 'NONE\n'
  exit 0
fi

python3 - "$PANEL_DETECT_MODE" "$PANEL_SERIAL_PATH" "$PANEL_SERIAL_PATH_RAW" "$PANEL_SERIAL_CANDIDATES_RAW" "$PANEL_I2C_BUS" "$PANEL_I2C_ADDR" <<'PY'
import glob
import os
import sys
import time

mode = sys.argv[1]
serial_path = sys.argv[2]
serial_path_raw = sys.argv[3]
serial_candidates_raw = sys.argv[4]
i2c_bus = int(sys.argv[5], 0)
i2c_addr = int(sys.argv[6], 0)

g2v1_found = False

KNOWN_USB_IDS = {
    (0x2341, 0x0058),  # Arduino Nano Every
    (0x2E8A, 0x0003),  # RP2040 Zero
}
KNOWN_UART_PATH_MARKERS = (
    "fe201600.serial",   # CM4 UART3
    "1f00038000.serial", # CM5 UART2
)

try:
    import serial  # type: ignore
    from serial.tools import list_ports  # type: ignore
except Exception:
    serial = None
    list_ports = None

try:
    from smbus import SMBus  # type: ignore
except Exception:
    try:
        from smbus2 import SMBus  # type: ignore
    except Exception:
        SMBus = None


def _add_candidate(candidates, path):
    if path and path not in candidates and os.path.exists(path):
        candidates.append(path)


def _is_known_uart_path(path):
    try:
        target = os.path.realpath(f"/sys/class/tty/{os.path.basename(path)}")
    except Exception:
        return False
    target = target.lower()
    return any(marker in target for marker in KNOWN_UART_PATH_MARKERS)


def _enumerate_pre_udev_candidates():
    candidates = []

    for raw_path in serial_candidates_raw.split(":"):
        _add_candidate(candidates, raw_path.strip())
    _add_candidate(candidates, serial_path_raw)

    if list_ports is not None:
        try:
            for port in list_ports.comports():
                vid = getattr(port, "vid", None)
                pid = getattr(port, "pid", None)
                if vid is not None and pid is not None and (vid, pid) in KNOWN_USB_IDS:
                    _add_candidate(candidates, port.device)
        except Exception:
            pass

    for pattern in ("/dev/ttyAMA*", "/dev/ttyS*"):
        for path in sorted(glob.glob(pattern)):
            if _is_known_uart_path(path):
                _add_candidate(candidates, path)

    for pattern in ("/dev/ttyUSB*", "/dev/ttyACM*"):
        for path in sorted(glob.glob(pattern)):
            _add_candidate(candidates, path)

    return candidates


def _detect_serial_panel(paths):
    if serial is None:
        return None

    for path in paths:
        ser = None
        try:
            ser = serial.Serial(path, 9600, timeout=1)
            for attempt in range(2):
                try:
                    ser.reset_input_buffer()
                except Exception:
                    pass
                ser.write(b"ZZZS;")
                ser.flush()
                response = ser.read_until(b";")
                # ZZZS08 identifies the current RemoteHead Arduino variant, but
                # for Saturn provisioning/LCD purposes this is still G2V2-class
                # front-panel hardware. RemoteHead should be modeled separately
                # as a system role, not as a front-panel type.
                if response.startswith((b"ZZZS05", b"ZZZS08")):
                    return "G2V2"
                if attempt == 0:
                    time.sleep(0.2)
        except Exception:
            pass
        finally:
            if ser is not None:
                try:
                    ser.close()
                except Exception:
                    pass
    return None


serial_result = None
if mode == "pre-udev":
    serial_result = _detect_serial_panel(_enumerate_pre_udev_candidates())
else:
    serial_result = _detect_serial_panel([serial_path] if serial_path else [])

if SMBus is not None:
    bus = None
    try:
        bus = SMBus(i2c_bus)
        g2v1_found = bus.read_byte_data(i2c_addr, 0x00) == 0xFF
    except Exception:
        pass
    finally:
        if bus is not None:
            try:
                bus.close()
            except Exception:
                pass

if serial_result == "G2V2":
    print("G2V2")
elif g2v1_found:
    print("G2V1")
else:
    print("NONE")
PY
