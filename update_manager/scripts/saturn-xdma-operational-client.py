#!/usr/bin/env python3
"""Dependency-free TCI acceptance client for the RX-only XDMA runtime."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import math
import os
import socket
import struct
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable
from urllib.parse import urlparse


HEADER_BYTES = 64
IQ_STREAM_TYPE = 0
AUDIO_STREAM_TYPE = 1
EXPECTED_IQ_RATE = 192_000
EXPECTED_AUDIO_RATE = 48_000
MAX_MESSAGE_BYTES = 4 * 1024 * 1024


class AcceptanceError(RuntimeError):
    pass


class WebSocket:
    def __init__(self, sock: socket.socket, buffered: bytes = b"") -> None:
        self.sock = sock
        self.buffered = bytearray(buffered)

    @classmethod
    def connect(cls, url: str, timeout: float) -> "WebSocket":
        parsed = urlparse(url)
        if parsed.scheme != "ws":
            raise AcceptanceError("only ws:// URLs are supported")
        if parsed.hostname not in {"127.0.0.1", "localhost", "::1"}:
            raise AcceptanceError("acceptance client is restricted to localhost")
        port = parsed.port or 80
        path = parsed.path or "/"
        if parsed.query:
            path = f"{path}?{parsed.query}"
        sock = socket.create_connection((parsed.hostname, port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        request = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {parsed.hostname}:{port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "\r\n"
        ).encode("ascii")
        sock.sendall(request)
        response = bytearray()
        while b"\r\n\r\n" not in response:
            chunk = sock.recv(4096)
            if not chunk:
                raise AcceptanceError("websocket closed during HTTP upgrade")
            response.extend(chunk)
            if len(response) > 65_536:
                raise AcceptanceError("websocket upgrade response is too large")
        header_end = response.index(b"\r\n\r\n") + 4
        header = response[:header_end].decode("iso-8859-1")
        lines = header.split("\r\n")
        if not lines or " 101 " not in f" {lines[0]} ":
            raise AcceptanceError(f"websocket upgrade failed: {lines[0] if lines else header!r}")
        headers: dict[str, str] = {}
        for line in lines[1:]:
            if ":" in line:
                name, value = line.split(":", 1)
                headers[name.strip().lower()] = value.strip()
        expected_accept = base64.b64encode(
            hashlib.sha1(
                (key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode("ascii")
            ).digest()
        ).decode("ascii")
        if headers.get("sec-websocket-accept") != expected_accept:
            raise AcceptanceError("websocket upgrade returned an invalid accept key")
        return cls(sock, bytes(response[header_end:]))

    def _recv_exact(self, length: int) -> bytes:
        while len(self.buffered) < length:
            chunk = self.sock.recv(max(4096, length - len(self.buffered)))
            if not chunk:
                raise AcceptanceError("websocket closed unexpectedly")
            self.buffered.extend(chunk)
        value = bytes(self.buffered[:length])
        del self.buffered[:length]
        return value

    def _recv_frame(self, timeout: float) -> tuple[bool, int, bytes]:
        self.sock.settimeout(timeout)
        first, second = self._recv_exact(2)
        final = bool(first & 0x80)
        if first & 0x70:
            raise AcceptanceError("websocket frame uses unsupported RSV bits")
        opcode = first & 0x0F
        masked = bool(second & 0x80)
        if masked:
            raise AcceptanceError("server websocket frame must not be masked")
        length = second & 0x7F
        if length == 126:
            length = struct.unpack("!H", self._recv_exact(2))[0]
        elif length == 127:
            length = struct.unpack("!Q", self._recv_exact(8))[0]
        if length > MAX_MESSAGE_BYTES:
            raise AcceptanceError(f"websocket frame exceeds {MAX_MESSAGE_BYTES} bytes")
        return final, opcode, self._recv_exact(length)

    def receive(self, timeout: float) -> str | bytes:
        fragments = bytearray()
        message_opcode: int | None = None
        while True:
            final, opcode, payload = self._recv_frame(timeout)
            if opcode == 0x8:
                raise AcceptanceError("server closed websocket before acceptance completed")
            if opcode == 0x9:
                self._send_frame(0xA, payload)
                continue
            if opcode == 0xA:
                continue
            if opcode in {0x1, 0x2}:
                if message_opcode is not None:
                    raise AcceptanceError("new websocket message started during fragmentation")
                message_opcode = opcode
                fragments.extend(payload)
            elif opcode == 0x0:
                if message_opcode is None:
                    raise AcceptanceError("unexpected websocket continuation frame")
                fragments.extend(payload)
            else:
                raise AcceptanceError(f"unsupported websocket opcode {opcode}")
            if len(fragments) > MAX_MESSAGE_BYTES:
                raise AcceptanceError(f"websocket message exceeds {MAX_MESSAGE_BYTES} bytes")
            if final:
                if message_opcode == 0x1:
                    return fragments.decode("utf-8")
                return bytes(fragments)

    def _send_frame(self, opcode: int, payload: bytes) -> None:
        mask = os.urandom(4)
        length = len(payload)
        header = bytearray([0x80 | opcode])
        if length < 126:
            header.append(0x80 | length)
        elif length <= 0xFFFF:
            header.append(0x80 | 126)
            header.extend(struct.pack("!H", length))
        else:
            header.append(0x80 | 127)
            header.extend(struct.pack("!Q", length))
        header.extend(mask)
        masked = bytes(value ^ mask[index % 4] for index, value in enumerate(payload))
        self.sock.sendall(bytes(header) + masked)

    def send_text(self, text: str) -> None:
        self._send_frame(0x1, text.encode("utf-8"))

    def close(self) -> None:
        try:
            self._send_frame(0x8, struct.pack("!H", 1000))
        except (AcceptanceError, OSError):
            pass
        self.sock.close()


@dataclass
class FrameStats:
    iq_frames: int = 0
    iq_pairs: int = 0
    audio_frames: int = 0
    audio_samples: int = 0
    iq_nonzero: bool = False
    audio_nonzero: bool = False


def parse_binary_frame(payload: bytes, stats: FrameStats) -> None:
    if len(payload) < HEADER_BYTES:
        raise AcceptanceError(f"TCI binary frame is only {len(payload)} bytes")
    sample_rate = struct.unpack_from("<I", payload, 4)[0]
    float_count = struct.unpack_from("<I", payload, 20)[0]
    frame_type = struct.unpack_from("<I", payload, 24)[0]
    channels = max(1, struct.unpack_from("<I", payload, 28)[0] or 2)
    expected_bytes = HEADER_BYTES + float_count * 4
    if float_count == 0 or len(payload) < expected_bytes:
        raise AcceptanceError(
            f"TCI frame payload is invalid: floats={float_count} bytes={len(payload)}"
        )
    probe_indexes = {0, float_count // 2, float_count - 1}
    values = [struct.unpack_from("<f", payload, HEADER_BYTES + index * 4)[0] for index in probe_indexes]
    if not all(math.isfinite(value) for value in values):
        raise AcceptanceError("TCI frame contains a non-finite sample")
    nonzero = any(abs(value) > 1e-12 for value in values)
    if frame_type == IQ_STREAM_TYPE:
        if sample_rate != EXPECTED_IQ_RATE or float_count % 2:
            raise AcceptanceError(
                f"unexpected IQ geometry: rate={sample_rate} floats={float_count}"
            )
        stats.iq_frames += 1
        stats.iq_pairs += float_count // 2
        stats.iq_nonzero = stats.iq_nonzero or nonzero
    elif frame_type == AUDIO_STREAM_TYPE:
        if sample_rate != EXPECTED_AUDIO_RATE or float_count % channels:
            raise AcceptanceError(
                f"unexpected audio geometry: rate={sample_rate} floats={float_count} channels={channels}"
            )
        stats.audio_frames += 1
        stats.audio_samples += float_count // channels
        stats.audio_nonzero = stats.audio_nonzero or nonzero


def read_readiness(path: Path) -> dict[str, object] | None:
    try:
        with path.open(encoding="utf-8") as handle:
            value = json.load(handle)
        return value if isinstance(value, dict) else None
    except (OSError, json.JSONDecodeError):
        return None


def wait_messages(
    websocket: WebSocket,
    stats: FrameStats,
    text_handler: Callable[[str], None],
    predicate: Callable[[], bool],
    timeout: float,
) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        remaining = max(0.05, min(0.5, deadline - time.monotonic()))
        try:
            message = websocket.receive(remaining)
        except socket.timeout:
            continue
        if isinstance(message, str):
            text_handler(message)
        else:
            parse_binary_frame(message, stats)
    if not predicate():
        raise AcceptanceError("timed out waiting for the expected TCI state")


def run_acceptance(url: str, readiness_file: Path, retune_hz: int, timeout: float) -> dict[str, object]:
    websocket = WebSocket.connect(url, timeout=min(timeout, 5.0))
    stats = FrameStats()
    bridge_ready = False
    remote_tx_disabled = False
    retune_vfo = False
    retune_dds = False
    tx_refused = False
    after_tx_request = False
    dsp_burst_continued = False

    def handle_text(text: str) -> None:
        nonlocal bridge_ready, remote_tx_disabled, retune_vfo, retune_dds, tx_refused
        bridge_ready = bridge_ready or "ready;" in text
        remote_tx_disabled = remote_tx_disabled or "remote_tx_rf_enabled:0,false;" in text
        retune_vfo = retune_vfo or f"vfo:0,0,{retune_hz};" in text
        retune_dds = retune_dds or f"dds:0,{retune_hz};" in text
        if after_tx_request and "trx:0,false;" in text:
            tx_refused = True

    try:
        wait_messages(
            websocket,
            stats,
            handle_text,
            lambda: bridge_ready and remote_tx_disabled,
            timeout=min(timeout, 8.0),
        )
        websocket.send_text(
            "iq_start:0;"
            "audio_samplerate:48000;"
            "audio_stream_samples:2048;"
            "audio_stream_channels:2;"
            "audio_start:0;"
        )
        wait_messages(
            websocket,
            stats,
            handle_text,
            lambda: stats.iq_frames >= 3 and stats.audio_frames >= 3,
            timeout=min(timeout, 10.0),
        )
        if not stats.iq_nonzero:
            raise AcceptanceError("IQ frames contain no measurable samples")

        before_dsp = read_readiness(readiness_file)
        before_dsp_dma = int(
            ((before_dsp or {}).get("metrics") or {}).get("dma_reads", 0)  # type: ignore[union-attr]
        )
        before_dsp_iq_frames = stats.iq_frames
        before_dsp_audio_frames = stats.audio_frames
        websocket.send_text(
            "modulation:0,USB;"
            "rx_filter_band:0,300,2700;"
            "rx_volume:0,0,-12.0;"
            "rx_nr_mode:0,OFF;"
            "rx_nb:0,OFF;"
            "rx_anf:0,false;"
            "rx_agc:0,FAST;"
            "rx_agc_gain:0,75;"
        )

        def dsp_stream_continued() -> bool:
            nonlocal dsp_burst_continued
            readiness = read_readiness(readiness_file)
            metrics = (readiness or {}).get("metrics")
            dsp_burst_continued = (
                stats.iq_frames >= before_dsp_iq_frames + 3
                and stats.audio_frames >= before_dsp_audio_frames + 3
                and isinstance(metrics, dict)
                and int(metrics.get("dma_reads", 0)) > before_dsp_dma
                and metrics.get("tx_capable") is False
                and metrics.get("rf_safe") is True
            )
            return dsp_burst_continued

        wait_messages(
            websocket,
            stats,
            handle_text,
            dsp_stream_continued,
            timeout=min(timeout, 8.0),
        )

        before_retune = read_readiness(readiness_file)
        before_dma = int(
            ((before_retune or {}).get("metrics") or {}).get("dma_reads", 0)  # type: ignore[union-attr]
        )
        websocket.send_text(f"vfo:0,0,{retune_hz};dds:0,{retune_hz};")

        def retune_ready() -> bool:
            readiness = read_readiness(readiness_file)
            metrics = (readiness or {}).get("metrics")
            return (
                retune_vfo
                and retune_dds
                and isinstance(metrics, dict)
                and metrics.get("frequency_hz") == retune_hz
                and int(metrics.get("dma_reads", 0)) > before_dma
                and metrics.get("tx_capable") is False
                and metrics.get("rf_safe") is True
            )

        wait_messages(
            websocket,
            stats,
            handle_text,
            retune_ready,
            timeout=min(timeout, 8.0),
        )

        after_tx_request = True
        websocket.send_text("trx:0,true,tci;")
        wait_messages(
            websocket,
            stats,
            handle_text,
            lambda: tx_refused,
            timeout=min(timeout, 5.0),
        )
        readiness = read_readiness(readiness_file) or {}
        metrics = readiness.get("metrics")
        if (
            readiness.get("rf_safe") is not True
            or not isinstance(metrics, dict)
            or metrics.get("rf_safe") is not True
            or metrics.get("tx_capable") is not False
        ):
            raise AcceptanceError("readiness did not remain receive-safe after TX refusal")

        websocket.send_text("trx:0,false;iq_stop:0;audio_stop:0;")
    finally:
        websocket.close()

    return {
        "status": "passed",
        "url": url,
        "retune_hz": retune_hz,
        "bridge_ready": bridge_ready,
        "remote_tx_rf_enabled": False,
        "tx_request_refused": tx_refused,
        "dsp_burst_continued": dsp_burst_continued,
        "iq_frames": stats.iq_frames,
        "iq_pairs": stats.iq_pairs,
        "iq_nonzero": stats.iq_nonzero,
        "audio_frames": stats.audio_frames,
        "audio_samples": stats.audio_samples,
        "audio_nonzero": stats.audio_nonzero,
    }


def self_test() -> None:
    payload = bytearray(HEADER_BYTES + 16)
    struct.pack_into("<I", payload, 4, EXPECTED_IQ_RATE)
    struct.pack_into("<I", payload, 20, 4)
    struct.pack_into("<I", payload, 24, IQ_STREAM_TYPE)
    struct.pack_into("<I", payload, 28, 2)
    struct.pack_into("<ffff", payload, HEADER_BYTES, 0.25, -0.25, 0.5, -0.5)
    stats = FrameStats()
    parse_binary_frame(bytes(payload), stats)
    if stats.iq_frames != 1 or stats.iq_pairs != 2 or not stats.iq_nonzero:
        raise AcceptanceError("binary frame self-test failed")
    print("saturn XDMA operational client self-test passed")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="ws://127.0.0.1:50001/")
    parser.add_argument("--readiness-file", type=Path)
    parser.add_argument("--retune-hz", type=int, default=7_200_000)
    parser.add_argument("--timeout-seconds", type=float, default=15.0)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.readiness_file is None:
        parser.error("--readiness-file is required unless --self-test is used")
    if not 100_000 <= args.retune_hz <= 61_440_000:
        parser.error("--retune-hz must be between 100 kHz and 61.44 MHz")
    try:
        result = run_acceptance(
            args.url,
            args.readiness_file,
            args.retune_hz,
            max(5.0, args.timeout_seconds),
        )
    except (AcceptanceError, OSError, ValueError) as error:
        print(f"saturn XDMA operational client FAILED: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
