#!/usr/bin/env python3
"""Dependency-free TCI acceptance client for the direct-XDMA runtime."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import math
import os
import select
import shlex
import socket
import ssl
import struct
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable
from urllib.parse import parse_qs, urlparse


HEADER_BYTES = 64
IQ_STREAM_TYPE = 0
AUDIO_STREAM_TYPE = 1
EXPECTED_IQ_RATE = 192_000
EXPECTED_AUDIO_RATE = 48_000
MAX_MESSAGE_BYTES = 4 * 1024 * 1024
TX_MIC_HEADER_BYTES = 64
TX_MIC_SAMPLE_RATE = 48_000
TX_MIC_BLOCK_SAMPLES = 1_024
TX_MIC_STREAM_TYPE = 2
TX_MIC_TONE_AMPLITUDE = 0.85


class AcceptanceError(RuntimeError):
    pass


class WebSocket:
    def __init__(self, sock: socket.socket, buffered: bytes = b"") -> None:
        self.sock = sock
        self.buffered = bytearray(buffered)

    @classmethod
    def connect(
        cls,
        url: str,
        timeout: float,
        *,
        authorization: str | None = None,
        insecure_tls: bool = False,
    ) -> "WebSocket":
        parsed = urlparse(url)
        if parsed.scheme not in {"ws", "wss"}:
            raise AcceptanceError("only ws:// and wss:// URLs are supported")
        if parsed.hostname not in {"127.0.0.1", "localhost", "::1"}:
            raise AcceptanceError("acceptance client is restricted to localhost")
        port = parsed.port or (443 if parsed.scheme == "wss" else 80)
        path = parsed.path or "/"
        if parsed.query:
            path = f"{path}?{parsed.query}"
        sock = socket.create_connection((parsed.hostname, port), timeout=timeout)
        if parsed.scheme == "wss":
            context = ssl.create_default_context()
            if insecure_tls:
                context.check_hostname = False
                context.verify_mode = ssl.CERT_NONE
            sock = context.wrap_socket(sock, server_hostname=parsed.hostname)
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        origin_scheme = "https" if parsed.scheme == "wss" else "http"
        authority = f"{parsed.hostname}:{port}"
        authorization_header = (
            f"Authorization: {authorization}\r\n" if authorization else ""
        )
        request = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {authority}\r\n"
            f"Origin: {origin_scheme}://{authority}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            f"{authorization_header}"
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

    def has_pending_data(self) -> bool:
        return bool(self.buffered) or (
            isinstance(self.sock, ssl.SSLSocket) and self.sock.pending() > 0
        )

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

    def send_binary(self, payload: bytes) -> None:
        self._send_frame(0x2, payload)

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


@dataclass
class LaneStats:
    control_text_messages: int = 0
    media_binary_messages: int = 0


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


def build_tx_mic_pcm_s16_frame(sequence: int, tone_hz: float = 1_000.0) -> bytes:
    payload_bytes = TX_MIC_BLOCK_SAMPLES * 2
    frame = bytearray(TX_MIC_HEADER_BYTES + payload_bytes)
    struct.pack_into("<I", frame, 4, TX_MIC_SAMPLE_RATE)
    struct.pack_into("<I", frame, 8, 1)  # s16
    struct.pack_into("<I", frame, 20, TX_MIC_BLOCK_SAMPLES)
    struct.pack_into("<I", frame, 24, TX_MIC_STREAM_TYPE)
    struct.pack_into("<I", frame, 28, 1)  # mono
    struct.pack_into("<I", frame, 32, sequence & 0xFFFF_FFFF)
    struct.pack_into("<I", frame, 36, 0)  # PCM
    struct.pack_into("<I", frame, 40, payload_bytes)
    first_sample = sequence * TX_MIC_BLOCK_SAMPLES
    for index in range(TX_MIC_BLOCK_SAMPLES):
        phase = 2.0 * math.pi * tone_hz * (first_sample + index) / TX_MIC_SAMPLE_RATE
        sample = round(math.sin(phase) * TX_MIC_TONE_AMPLITUDE * 32767.0)
        struct.pack_into("<h", frame, TX_MIC_HEADER_BYTES + index * 2, sample)
    return bytes(frame)


def wait_messages(
    websockets: list[WebSocket],
    stats: FrameStats,
    lanes: LaneStats,
    text_handler: Callable[[str], None],
    predicate: Callable[[], bool],
    timeout: float,
) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        remaining = max(0.05, min(0.5, deadline - time.monotonic()))
        ready = [websocket for websocket in websockets if websocket.has_pending_data()]
        if not ready:
            readable, _, _ = select.select(
                [websocket.sock for websocket in websockets], [], [], remaining
            )
            ready = [
                websocket for websocket in websockets if websocket.sock in readable
            ]
        if not ready:
            continue
        try:
            source = ready[0]
            message = source.receive(remaining)
        except socket.timeout:
            continue
        if isinstance(message, str):
            if len(websockets) > 1 and source is not websockets[0]:
                raise AcceptanceError("split media lane delivered a text message")
            lanes.control_text_messages += 1
            text_handler(message)
        else:
            if len(websockets) > 1 and source is not websockets[1]:
                raise AcceptanceError("split control lane delivered a binary message")
            lanes.media_binary_messages += 1
            parse_binary_frame(message, stats)
    if not predicate():
        raise AcceptanceError("timed out waiting for the expected TCI state")


def basic_auth_header(spec: str) -> str:
    if ":" not in spec:
        raise AcceptanceError("basic authentication must use username:password format")
    username, password = spec.split(":", 1)
    if not username or not password:
        raise AcceptanceError("basic authentication username and password must be non-empty")
    encoded = base64.b64encode(spec.encode("utf-8")).decode("ascii")
    return f"Basic {encoded}"


def systemd_basic_auth(unit: str) -> str:
    if (
        not unit.endswith(".service")
        or not unit
        or any(
            character not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_.@-"
            for character in unit
        )
    ):
        raise AcceptanceError("unsafe systemd unit name for credential lookup")
    result = subprocess.run(
        ["systemctl", "show", "--property=Environment", "--value", unit],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise AcceptanceError(f"could not inspect {unit} environment")
    try:
        entries = shlex.split(result.stdout)
    except ValueError as error:
        raise AcceptanceError(f"could not parse {unit} environment") from error
    for entry in entries:
        if entry.startswith("SATURN_REMOTE_BASIC_AUTH="):
            return entry.split("=", 1)[1]
    raise AcceptanceError(f"{unit} does not expose SATURN_REMOTE_BASIC_AUTH")


def run_acceptance(
    url: str,
    readiness_file: Path,
    retune_hz: int,
    timeout: float,
    *,
    media_url: str | None = None,
    authorization: str | None = None,
    insecure_tls: bool = False,
    rf_tx_probe: bool = False,
    tx_duration_ms: int = 2_500,
    tx_drive_watts: int = 3,
) -> dict[str, object]:
    control = WebSocket.connect(
        url,
        timeout=min(timeout, 5.0),
        authorization=authorization,
        insecure_tls=insecure_tls,
    )
    media = (
        WebSocket.connect(
            media_url,
            timeout=min(timeout, 5.0),
            authorization=authorization,
            insecure_tls=insecure_tls,
        )
        if media_url
        else None
    )
    websockets = [control] + ([media] if media is not None else [])
    stats = FrameStats()
    lanes = LaneStats()
    bridge_ready = False
    remote_tx_state_seen = False
    split_session = (
        parse_qs(urlparse(url).query).get("session", [""])[0] if media_url else ""
    )
    split_paired = not media_url
    retune_vfo = False
    retune_dds = False
    tx_release_confirmed = False
    after_tx_request = False
    dsp_burst_continued = False
    rf_inhibited_duc_exercised = False

    def handle_text(text: str) -> None:
        nonlocal bridge_ready, remote_tx_state_seen, split_paired
        nonlocal retune_vfo, retune_dds, tx_release_confirmed
        bridge_ready = bridge_ready or "ready;" in text
        expected_rf = "true" if rf_tx_probe else "false"
        remote_tx_state_seen = remote_tx_state_seen or (
            f"remote_tx_rf_enabled:0,{expected_rf};" in text
        )
        if split_session and f"session_paired:{split_session};" in text:
            split_paired = True
        retune_vfo = retune_vfo or f"vfo:0,0,{retune_hz};" in text
        retune_dds = retune_dds or f"dds:0,{retune_hz};" in text
        if after_tx_request and "trx:0,false;" in text:
            tx_release_confirmed = True

    try:
        if media is not None:
            control.send_text(f"session_open:{split_session},operator;")
            # The proxy injects lane metadata independently on both sockets.
            # Give the bridge a bounded interval to register the pair before
            # stream-enable commands are mirrored to the media lane.
            pair_settle_deadline = time.monotonic() + 0.5
            wait_messages(
                websockets,
                stats,
                lanes,
                handle_text,
                lambda: time.monotonic() >= pair_settle_deadline,
                timeout=1.0,
            )
        wait_messages(
            websockets,
            stats,
            lanes,
            handle_text,
            lambda: bridge_ready and remote_tx_state_seen,
            timeout=min(timeout, 8.0),
        )
        control.send_text(
            "iq_start:0;"
            "audio_samplerate:48000;"
            "audio_stream_samples:2048;"
            "audio_stream_channels:2;"
            "audio_start:0;"
        )
        wait_messages(
            websockets,
            stats,
            lanes,
            handle_text,
            lambda: stats.iq_frames >= 3 and stats.audio_frames >= 3,
            timeout=min(timeout, 10.0),
        )
        split_paired = split_paired or (
            lanes.control_text_messages > 0 and lanes.media_binary_messages > 0
        )
        if not split_paired:
            raise AcceptanceError("split control/media lane routing was not observed")
        if not stats.iq_nonzero:
            raise AcceptanceError("IQ frames contain no measurable samples")

        before_dsp = read_readiness(readiness_file)
        before_dsp_dma = int(
            ((before_dsp or {}).get("metrics") or {}).get("dma_reads", 0)  # type: ignore[union-attr]
        )
        before_dsp_iq_frames = stats.iq_frames
        before_dsp_audio_frames = stats.audio_frames
        control.send_text(
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
                and metrics.get("tx_capable") is True
                and metrics.get("rf_safe") is True
            )
            return dsp_burst_continued

        wait_messages(
            websockets,
            stats,
            lanes,
            handle_text,
            dsp_stream_continued,
            timeout=min(timeout, 8.0),
        )

        before_retune = read_readiness(readiness_file)
        before_dma = int(
            ((before_retune or {}).get("metrics") or {}).get("dma_reads", 0)  # type: ignore[union-attr]
        )
        control.send_text(f"vfo:0,0,{retune_hz};dds:0,{retune_hz};")

        def retune_ready() -> bool:
            readiness = read_readiness(readiness_file)
            metrics = (readiness or {}).get("metrics")
            return (
                retune_vfo
                and retune_dds
                and isinstance(metrics, dict)
                and metrics.get("frequency_hz") == retune_hz
                and int(metrics.get("dma_reads", 0)) > before_dma
                and metrics.get("tx_capable") is True
                and metrics.get("rf_safe") is True
            )

        wait_messages(
            websockets,
            stats,
            lanes,
            handle_text,
            retune_ready,
            timeout=min(timeout, 8.0),
        )

        before_tx = read_readiness(readiness_file) or {}
        before_tx_metrics = before_tx.get("metrics") or {}
        before_tx_writes = int(before_tx_metrics.get("tx_dma_writes", 0))
        before_tx_frames = int(before_tx_metrics.get("tx_frames", 0))
        tx_keyed_observed = False
        peak_forward_watts = 0.0
        peak_reverse_watts = 0.0
        peak_swr = 1.0
        rf_tx_exercised = False
        after_tx_request = True

        if rf_tx_probe:
            control.send_text(
                f"tx_drive:0,{tx_drive_watts};"
                "tx_mic_gain:0,0.0;"
                "tx_filter_band:0,50,3800;"
                "tx_eq_enable:0,false;"
                "tx_cfc_enable:0,false;"
                "iq_stop:0;audio_stop:0;"
                "trx:0,true,tci;"
            )
            tx_socket = media if media is not None else control
            sequence = 1
            started_tx = time.monotonic()
            deadline_tx = started_tx + tx_duration_ms / 1000.0
            next_frame_at = started_tx
            next_heartbeat_at = started_tx
            frame_period = TX_MIC_BLOCK_SAMPLES / TX_MIC_SAMPLE_RATE
            try:
                while time.monotonic() < deadline_tx:
                    now = time.monotonic()
                    if now >= next_frame_at:
                        tx_socket.send_binary(build_tx_mic_pcm_s16_frame(sequence))
                        sequence += 1
                        next_frame_at += frame_period
                    if now >= next_heartbeat_at:
                        control.send_text(f"saturn_ping:rf-probe-{sequence};")
                        next_heartbeat_at += 0.25
                    current = read_readiness(readiness_file) or {}
                    current_metrics = current.get("metrics") or {}
                    tx_keyed_observed = tx_keyed_observed or (
                        current_metrics.get("tx_keyed") is True
                    )
                    peak_forward_watts = max(
                        peak_forward_watts,
                        float(current_metrics.get("forward_watts", 0.0)),
                    )
                    peak_reverse_watts = max(
                        peak_reverse_watts,
                        float(current_metrics.get("reverse_watts", 0.0)),
                    )
                    peak_swr = max(peak_swr, float(current_metrics.get("swr", 1.0)))
                    if int(current_metrics.get("tx_fifo_faults", 0)) != 0:
                        raise AcceptanceError("production TX reported a FIFO fault")
                    time.sleep(max(0.001, min(0.005, next_frame_at - time.monotonic())))
            finally:
                control.send_text("trx:0,false;")

            def rf_release_ready() -> bool:
                current = read_readiness(readiness_file) or {}
                current_metrics = current.get("metrics") or {}
                return (
                    current.get("rf_safe") is True
                    and current_metrics.get("rf_safe") is True
                    and current_metrics.get("tx_keyed") is False
                    and int(current_metrics.get("tx_frames", 0)) > before_tx_frames
                    and int(current_metrics.get("tx_fifo_faults", 0)) == 0
                )

            wait_messages(
                websockets,
                stats,
                lanes,
                handle_text,
                rf_release_ready,
                timeout=min(timeout, 8.0),
            )
            final_tx = read_readiness(readiness_file) or {}
            final_tx_metrics = final_tx.get("metrics") or {}
            rf_tx_exercised = (
                tx_keyed_observed
                and peak_forward_watts >= 0.05
                and peak_forward_watts <= 4.0
                and peak_reverse_watts <= 0.75
                and (peak_forward_watts < 0.25 or peak_swr <= 3.0)
                and int(final_tx_metrics.get("tx_dma_writes", 0)) > before_tx_writes
                and int(final_tx_metrics.get("tx_frames", 0)) > before_tx_frames
                and int(final_tx_metrics.get("tx_fifo_faults", 0)) == 0
            )
            if not rf_tx_exercised:
                raise AcceptanceError(
                    "production RF evidence was incomplete: "
                    f"keyed={tx_keyed_observed} forward={peak_forward_watts:.3f}W "
                    f"reverse={peak_reverse_watts:.3f}W swr={peak_swr:.2f}"
                )
            control.send_text("iq_stop:0;audio_stop:0;")
        else:
            control.send_text("tx_two_tone:0,true;trx:0,true,tci;")

            def rf_inhibited_duc_ready() -> bool:
                nonlocal rf_inhibited_duc_exercised
                current = read_readiness(readiness_file) or {}
                current_metrics = current.get("metrics")
                rf_inhibited_duc_exercised = (
                    current.get("rf_safe") is True
                    and isinstance(current_metrics, dict)
                    and current_metrics.get("rf_safe") is True
                    and current_metrics.get("tx_rf_enabled") is False
                    and current_metrics.get("tx_stream_active") is True
                    and current_metrics.get("tx_keyed") is False
                    and int(current_metrics.get("tx_dma_writes", 0)) > before_tx_writes
                    and int(current_metrics.get("tx_frames", 0)) >= 20
                    and int(current_metrics.get("tx_fifo_faults", 0)) == 0
                )
                return rf_inhibited_duc_exercised

            wait_messages(
                websockets,
                stats,
                lanes,
                handle_text,
                rf_inhibited_duc_ready,
                timeout=min(timeout, 8.0),
            )
            readiness = read_readiness(readiness_file) or {}
            metrics = readiness.get("metrics")
            if (
                readiness.get("rf_safe") is not True
                or not isinstance(metrics, dict)
                or metrics.get("rf_safe") is not True
                or metrics.get("tx_capable") is not True
            ):
                raise AcceptanceError(
                    "readiness did not remain receive-safe during RF-inhibited TX"
                )
            control.send_text(
                "trx:0,false;tx_two_tone:0,false;iq_stop:0;audio_stop:0;"
            )
    finally:
        if media is not None:
            media.close()
        control.close()

    return {
        "status": "passed",
        "url": url,
        "transport": "split-proxy" if media_url else "direct",
        "retune_hz": retune_hz,
        "bridge_ready": bridge_ready,
        "split_paired": split_paired,
        "control_text_messages": lanes.control_text_messages,
        "media_binary_messages": lanes.media_binary_messages,
        "remote_tx_rf_enabled": rf_tx_probe,
        "tx_release_confirmed": tx_release_confirmed,
        "rf_inhibited_duc_exercised": rf_inhibited_duc_exercised,
        "rf_tx_exercised": rf_tx_exercised,
        "tx_keyed_observed": tx_keyed_observed,
        "tx_duration_ms": tx_duration_ms if rf_tx_probe else 0,
        "tx_drive_watts": tx_drive_watts if rf_tx_probe else 0,
        "peak_forward_watts": peak_forward_watts,
        "peak_reverse_watts": peak_reverse_watts,
        "peak_swr": peak_swr,
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
    if basic_auth_header("admin:secret") != "Basic YWRtaW46c2VjcmV0":
        raise AcceptanceError("basic authentication self-test failed")
    mic = build_tx_mic_pcm_s16_frame(7)
    if (
        len(mic) != TX_MIC_HEADER_BYTES + TX_MIC_BLOCK_SAMPLES * 2
        or struct.unpack_from("<I", mic, 24)[0] != TX_MIC_STREAM_TYPE
        or struct.unpack_from("<I", mic, 32)[0] != 7
    ):
        raise AcceptanceError("TX microphone frame self-test failed")
    print("saturn XDMA operational client self-test passed")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="ws://127.0.0.1:50001/")
    parser.add_argument("--media-url")
    parser.add_argument("--readiness-file", type=Path)
    parser.add_argument("--retune-hz", type=int, default=7_200_000)
    parser.add_argument("--timeout-seconds", type=float, default=15.0)
    parser.add_argument("--basic-auth-systemd-unit")
    parser.add_argument("--insecure-tls", action="store_true")
    parser.add_argument("--rf-tx-probe", action="store_true")
    parser.add_argument("--tx-duration-ms", type=int, default=2_500)
    parser.add_argument("--tx-drive-watts", type=int, default=3)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.readiness_file is None:
        parser.error("--readiness-file is required unless --self-test is used")
    if not 100_000 <= args.retune_hz <= 61_440_000:
        parser.error("--retune-hz must be between 100 kHz and 61.44 MHz")
    if args.rf_tx_probe and args.retune_hz != 7_200_000:
        parser.error("--rf-tx-probe is locked to 7.200 MHz")
    if args.rf_tx_probe and not 500 <= args.tx_duration_ms <= 3_000:
        parser.error("--rf-tx-probe duration must be 500..3000 ms")
    if args.rf_tx_probe and args.tx_drive_watts != 3:
        parser.error("--rf-tx-probe is locked to 3 W")
    control_url = urlparse(args.url)
    if bool(args.media_url) != (control_url.path == "/saturn/control"):
        parser.error("--media-url must be paired with a /saturn/control control URL")
    if args.media_url:
        media_url = urlparse(args.media_url)
        if (
            media_url.scheme != control_url.scheme
            or media_url.netloc != control_url.netloc
            or media_url.path != "/saturn/media"
            or parse_qs(media_url.query).get("session")
            != parse_qs(control_url.query).get("session")
        ):
            parser.error(
                "split control/media URLs must share an authority and session"
            )
    try:
        auth_spec = os.environ.get("SATURN_REMOTE_BASIC_AUTH")
        if args.basic_auth_systemd_unit:
            auth_spec = auth_spec or systemd_basic_auth(args.basic_auth_systemd_unit)
        authorization = basic_auth_header(auth_spec) if auth_spec else None
        result = run_acceptance(
            args.url,
            args.readiness_file,
            args.retune_hz,
            max(5.0, args.timeout_seconds),
            media_url=args.media_url,
            authorization=authorization,
            insecure_tls=args.insecure_tls,
            rf_tx_probe=args.rf_tx_probe,
            tx_duration_ms=args.tx_duration_ms,
            tx_drive_watts=args.tx_drive_watts,
        )
    except (AcceptanceError, OSError, ValueError) as error:
        print(f"saturn XDMA operational client FAILED: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
