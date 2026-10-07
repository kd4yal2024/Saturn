#!/usr/bin/env python3
"""Fail closed on RXC1 primary-image identity, BIT structure, and BIN export."""

import argparse
import hashlib
import json
import struct
from pathlib import Path

PRIMARY_CAPACITY = 0x01300000 - 0x00980000
BIT_MAGIC = bytes.fromhex("0ff00ff00ff00ff000")
SYNC = bytes.fromhex("aa995566")


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_bit(path: Path) -> tuple[dict[str, str], bytes]:
    data = path.read_bytes()
    if len(data) < 2:
        raise ValueError("BIT header is truncated")
    magic_length = struct.unpack_from(">H", data)[0]
    offset = 2
    if data[offset : offset + magic_length] != BIT_MAGIC:
        raise ValueError("BIT header magic differs")
    offset += magic_length
    if data[offset : offset + 2] != b"\x00\x01":
        raise ValueError("BIT header marker differs")
    offset += 2
    fields = {}
    for key in "abcd":
        if data[offset : offset + 1] != key.encode():
            raise ValueError(f"missing BIT field {key}")
        offset += 1
        size = struct.unpack_from(">H", data, offset)[0]
        offset += 2
        fields[key] = data[offset : offset + size].rstrip(b"\0").decode("ascii")
        offset += size
    if data[offset : offset + 1] != b"e":
        raise ValueError("missing BIT payload")
    payload_size = struct.unpack_from(">I", data, offset + 1)[0]
    payload = data[offset + 5 :]
    if len(payload) != payload_size or len(payload) % 4:
        raise ValueError("BIT payload length differs")
    return fields, payload


def inspect_packets(payload: bytes) -> tuple[list[int], int]:
    sync = payload.find(SYNC)
    if sync < 0 or sync % 4:
        raise ValueError("missing aligned configuration sync")
    offset = sync + 4
    last_register = None
    axss = []
    fdri_words = 0
    while offset < len(payload):
        header = struct.unpack_from(">I", payload, offset)[0]
        packet_type = header >> 29
        if packet_type == 1:
            operation = (header >> 27) & 3
            register = (header >> 13) & 0x3FFF
            count = header & 0x7FF
            last_register = register
        elif packet_type == 2 and last_register is not None:
            operation = (header >> 27) & 3
            register = last_register
            count = header & 0x7FFFFFF
        else:
            raise ValueError(f"invalid configuration packet at {offset}")
        offset += 4
        if count > (len(payload) - offset) // 4:
            raise ValueError("configuration packet exceeds payload")
        if operation == 2 and register == 13:
            if count != 1:
                raise ValueError("AXSS write is not one word")
            axss.append(struct.unpack_from(">I", payload, offset)[0])
        if operation == 2 and register == 2:
            fdri_words += count
        offset += 4 * count
    if len(axss) != 1 or fdri_words == 0:
        raise ValueError("missing unique AXSS or FDRI configuration frames")
    return axss, fdri_words


def verify(bitfile: Path, binfile: Path, expected_id: int) -> dict:
    fields, payload = read_bit(bitfile)
    bin_data = binfile.read_bytes()
    if not fields["a"].startswith("saturn_top_wrapper;") or fields["b"] != "7a200tfbg676":
        raise ValueError("unexpected design or FPGA part")
    if len(bin_data) == 0 or len(bin_data) > PRIMARY_CAPACITY or bin_data != payload:
        raise ValueError("BIN differs from BIT payload or exceeds primary slot")
    axss, fdri_words = inspect_packets(payload)
    if axss != [expected_id]:
        raise ValueError(f"unexpected embedded USR_ACCESS: {[f'0x{value:08X}' for value in axss]}")
    return {
        "status": "PASS",
        "bit": str(bitfile),
        "bit_sha256": digest(bitfile.read_bytes()),
        "bin": str(binfile),
        "bin_sha256": digest(bin_data),
        "bin_bytes": len(bin_data),
        "primary_capacity_bytes": PRIMARY_CAPACITY,
        "embedded_usr_access": f"0x{expected_id:08X}",
        "configuration_frame_words": fdri_words,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bit", type=Path)
    parser.add_argument("bin", type=Path)
    parser.add_argument("--build-id", required=True, type=lambda value: int(value, 0))
    args = parser.parse_args()
    print(json.dumps(verify(args.bit, args.bin, args.build_id), indent=2))


if __name__ == "__main__":
    main()
