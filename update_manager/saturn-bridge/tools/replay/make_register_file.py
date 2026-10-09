#!/usr/bin/env python3
"""Create the fake XDMA register space used by run_replay_bridge.sh.

The Bridge reads and writes its FPGA registers with positional 32-bit
little-endian accesses, so an ordinary file works: writes persist, which is
all the Bridge's own read-back checks need. The identity registers present the
identified 1.31 baseline image (USR_ACCESS 0x53460003) on a primary PCB2, the
firmware the G2 reports. The DDC FIFO status register is a constant depth of
512 words, which makes the Bridge's reader issue 4096-byte reads, about 844 a
second at 384 kS/s, the same cadence as the real device. Nothing else is
emulated: read-to-clear and counter registers stay zero.
"""
import struct
import sys

SIZE = 0x10000
REGISTERS = {
    0x4004: 0x5346_0003,                                            # user version / build id
    0x9000: 512,                                                    # DDC FIFO monitor: depth only
    0xC000: (1 << 25) | (4 << 20) | (31 << 4) | 0xF,                # firmware 1.31, primary image, clocks 0xF
    0xC004: (1 << 16) | 2,                                          # Saturn product, PCB 2
}


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: make_register_file.py OUTPUT", file=sys.stderr)
        return 2
    data = bytearray(SIZE)
    for offset, value in REGISTERS.items():
        data[offset:offset + 4] = struct.pack("<I", value)
    with open(sys.argv[1], "wb") as out:
        out.write(data)
    return 0


if __name__ == "__main__":
    sys.exit(main())
