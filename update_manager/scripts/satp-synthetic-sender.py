#!/usr/bin/env python3
"""Deterministic SATP v1 burst sender for the Phase 0E null-sink soak."""

import argparse
import math
import random
import socket
import struct
import time

RATE = 48_000
FRAMES = 128
PACKETS_PER_CALLBACK = 4
CALLBACK_FRAMES = FRAMES * PACKETS_PER_CALLBACK
CALLBACK_PERIOD = CALLBACK_FRAMES / RATE
HEADER = struct.Struct("<4sBBBBIIIQHH")


def args():
    parser = argparse.ArgumentParser()
    parser.add_argument("host")
    parser.add_argument("--port", type=int, default=50100)
    parser.add_argument("--seconds", type=float, default=30 * 60)
    parser.add_argument("--session", type=lambda value: int(value, 0), default=None)
    parser.add_argument("--stream", type=lambda value: int(value, 0), default=0)
    parser.add_argument("--frequency", type=float, default=1000.0)
    parser.add_argument("--amplitude", type=float, default=0.1)
    return parser.parse_args()


def main():
    options = args()
    if not 1 <= options.port <= 65535:
        raise SystemExit("port must be 1-65535")
    if options.seconds <= 0:
        raise SystemExit("seconds must be positive")
    if not 0.0 <= options.amplitude <= 1.0:
        raise SystemExit("amplitude must be 0-1")

    session = options.session
    if session is None:
        session = random.SystemRandom().randrange(1, 2**32)
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sequence = 0
    sample_counter = 0
    packets = 0
    started = time.monotonic()
    deadline = started
    stop_at = started + options.seconds

    while time.monotonic() < stop_at:
        for _ in range(PACKETS_PER_CALLBACK):
            header = HEADER.pack(
                b"SAT1",
                1,
                1,  # TX_AUDIO
                1,  # mono
                1,  # FLOAT32_LE
                session,
                options.stream,
                sequence,
                sample_counter,
                FRAMES,
                0,
            )
            samples = [
                options.amplitude
                * math.sin(2.0 * math.pi * options.frequency * (sample_counter + i) / RATE)
                for i in range(FRAMES)
            ]
            sock.sendto(header + struct.pack("<128f", *samples), (options.host, options.port))
            sequence = (sequence + 1) & 0xFFFFFFFF
            sample_counter = (sample_counter + FRAMES) & 0xFFFFFFFFFFFFFFFF
            packets += 1
        deadline += CALLBACK_PERIOD
        delay = deadline - time.monotonic()
        if delay > 0:
            time.sleep(delay)

    elapsed = time.monotonic() - started
    print(
        f"session={session} packets={packets} frames={packets * FRAMES} "
        f"elapsed={elapsed:.3f}s packet_rate={packets / elapsed:.3f}/s"
    )


if __name__ == "__main__":
    main()
