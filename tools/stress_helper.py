#!/usr/bin/env python3
"""Exercise one Omnivox capture helper repeatedly in a single process.

The test validates protocol ordering, PCM framing, marker bounds, health pings,
and clean shutdown.  It uses only the Python standard library so it can run
from WSL against the Windows helper executables.
"""

import argparse
import base64
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import threading
import time


PROTOCOL_VERSION = 2
FRAME_TIMEOUT_SECONDS = 30.0
TEST_TEXTS = (
    "First sentence has several words. Second sentence checks completion!",
    "Unicode café and naïve words work here. Another sentence follows?",
    "A short clause, followed by another clause; then the sentence ends.",
)


class HelperSession:
    def __init__(self, command):
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
        )
        self.responses = queue.Queue()
        self.stderr_lines = []
        self.stdout_thread = threading.Thread(target=self._read_stdout, daemon=True)
        self.stderr_thread = threading.Thread(target=self._read_stderr, daemon=True)
        self.stdout_thread.start()
        self.stderr_thread.start()

    def _read_stdout(self):
        for line in self.process.stdout:
            self.responses.put(line)
        self.responses.put(None)

    def _read_stderr(self):
        for line in self.process.stderr:
            if len(self.stderr_lines) < 100:
                self.stderr_lines.append(line.rstrip())

    def send(self, request):
        if self.process.poll() is not None:
            raise RuntimeError(f"helper exited with status {self.process.returncode}")
        self.process.stdin.write(json.dumps(request, separators=(",", ":")) + "\n")
        self.process.stdin.flush()

    def receive(self, request_id, timeout=FRAME_TIMEOUT_SECONDS):
        try:
            line = self.responses.get(timeout=timeout)
        except queue.Empty as error:
            raise RuntimeError(f"timed out waiting for helper request {request_id}") from error
        if line is None:
            raise RuntimeError(
                f"helper closed stdout with status {self.process.poll()}; "
                f"stderr: {self.stderr_lines[-5:]}"
            )
        try:
            response = json.loads(line)
        except json.JSONDecodeError as error:
            raise RuntimeError(f"helper emitted invalid JSON: {line[:200]!r}") from error
        if response.get("protocol_version") != PROTOCOL_VERSION:
            raise RuntimeError(f"unexpected protocol version: {response}")
        if response.get("request_id") != request_id:
            raise RuntimeError(f"unexpected request ID: {response}")
        if response.get("type") == "error":
            raise RuntimeError(
                f"helper error {response.get('code')}: {response.get('message')}"
            )
        return response

    def stop(self):
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=10)


def request(request_id, request_type, **fields):
    return {
        "protocol_version": PROTOCOL_VERSION,
        "request_id": request_id,
        "type": request_type,
        **fields,
    }


def resident_bytes(process_id):
    """Return observable launcher RSS on procfs systems, or None."""
    status = Path(f"/proc/{process_id}/status")
    try:
        for line in status.read_text(encoding="utf-8").splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) * 1024
    except (OSError, ValueError):
        return None
    return None


def validate_marker(marker, frame_count, text_size):
    offset = marker.get("frame_offset")
    if not isinstance(offset, int) or offset < 0 or offset > frame_count:
        raise RuntimeError(f"marker frame is outside synthesized audio: {marker}")
    start = marker.get("text_start")
    length = marker.get("text_length")
    if (start is None) != (length is None):
        raise RuntimeError(f"marker has a partial source range: {marker}")
    if start is not None and (
        not isinstance(start, int)
        or not isinstance(length, int)
        or start < 0
        or length < 0
        or start + length > text_size
    ):
        raise RuntimeError(f"marker source range is invalid: {marker}")


def synthesize(session, request_id, text, voice_id, iteration):
    settings = {
        "voice_id": voice_id,
        "rate": (0.35, 0.5, 0.7)[iteration % 3],
        "pitch": (0.8, 1.0, 1.2)[iteration % 3],
        "volume": (0.35, 0.65, 1.0)[iteration % 3],
    }
    session.send(
        request(
            request_id,
            "synthesize",
            text=text,
            settings=settings,
            anchors=[
                {
                    "id": f"start-{iteration}",
                    "text_offset": 0,
                    "affinity": "before",
                }
            ],
        )
    )

    started = False
    channels = None
    next_sequence = 0
    audio_bytes = 0
    markers = []
    while True:
        response = session.receive(request_id)
        response_type = response.get("type")
        if response_type == "synthesis_started":
            if started:
                raise RuntimeError("helper emitted duplicate synthesis_started")
            started = True
            audio_format = response.get("format", {})
            channels = audio_format.get("channels")
            if (
                audio_format.get("sample_rate", 0) <= 0
                or channels not in (1, 2)
                or audio_format.get("sample_format") != "pcm_s16_le"
            ):
                raise RuntimeError(f"helper advertised an invalid audio format: {response}")
            if response.get("actual_voice_id") != voice_id:
                raise RuntimeError(f"helper realized the wrong voice: {response}")
        elif response_type == "audio_chunk":
            chunk = response.get("chunk", {})
            if not started or chunk.get("sequence") != next_sequence:
                raise RuntimeError(f"helper emitted an out-of-order audio chunk: {response}")
            try:
                decoded = base64.b64decode(chunk.get("data_base64", ""), validate=True)
            except (ValueError, TypeError) as error:
                raise RuntimeError("helper emitted invalid Base64 PCM") from error
            if len(decoded) % (channels * 2) != 0:
                raise RuntimeError("helper emitted a partial PCM frame")
            audio_bytes += len(decoded)
            next_sequence += 1
        elif response_type == "markers":
            values = response.get("markers")
            if not isinstance(values, list):
                raise RuntimeError(f"helper emitted an invalid marker batch: {response}")
            markers.extend(values)
        elif response_type == "synthesis_completed":
            if not started:
                raise RuntimeError("helper completed before synthesis_started")
            frame_count = response.get("frame_count")
            if frame_count != audio_bytes // (channels * 2) or frame_count <= 0:
                raise RuntimeError(
                    f"helper frame total does not match PCM: {frame_count}, {audio_bytes} bytes"
                )
            text_size = len(text.encode("utf-8"))
            for marker in markers:
                validate_marker(marker, frame_count, text_size)
            kinds = {marker.get("kind") for marker in markers}
            if "word" not in kinds or "sentence" not in kinds:
                raise RuntimeError(f"helper omitted expected word/sentence markers: {kinds}")
            return frame_count, len(markers), audio_bytes
        else:
            raise RuntimeError(f"unexpected synthesis response: {response}")


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("helper", help="path to an Omnivox helper executable")
    parser.add_argument("--engine-id", required=True, help="expected descriptor engine ID")
    parser.add_argument("--voice-id", help="voice to exercise; defaults to helper default")
    parser.add_argument("--iterations", type=int, default=100)
    parser.add_argument(
        "--helper-arg",
        action="append",
        default=[],
        help="additional helper argument, such as an explicit native DLL path",
    )
    return parser.parse_args()


def main():
    args = parse_args()
    if args.iterations <= 0:
        raise SystemExit("--iterations must be positive")
    command = [args.helper, *args.helper_arg]
    session = HelperSession(command)
    started_at = time.monotonic()
    initial_rss = resident_bytes(session.process.pid)
    peak_rss = initial_rss
    total_frames = 0
    total_markers = 0
    total_bytes = 0
    try:
        session.send(
            request(1, "hello", supported_protocol_versions=[PROTOCOL_VERSION, 1])
        )
        hello = session.receive(1)
        if hello.get("type") != "hello" or hello.get("selected_protocol_version") != 2:
            raise RuntimeError(f"helper failed protocol negotiation: {hello}")

        session.send(request(2, "describe"))
        described = session.receive(2)
        descriptor = described.get("descriptor", {})
        if described.get("type") != "descriptor" or descriptor.get("id") != args.engine_id:
            raise RuntimeError(f"helper returned the wrong descriptor: {described}")
        voice_id = args.voice_id or descriptor.get("default_voice_id")
        if not voice_id:
            raise RuntimeError("helper descriptor has no usable default voice")

        next_request_id = 3
        for iteration in range(args.iterations):
            text = TEST_TEXTS[iteration % len(TEST_TEXTS)]
            frames, markers, byte_count = synthesize(
                session,
                next_request_id,
                text,
                voice_id,
                iteration,
            )
            next_request_id += 1
            total_frames += frames
            total_markers += markers
            total_bytes += byte_count
            current_rss = resident_bytes(session.process.pid)
            if current_rss is not None:
                peak_rss = max(peak_rss or current_rss, current_rss)

            if (iteration + 1) % 25 == 0:
                session.send(request(next_request_id, "ping"))
                pong = session.receive(next_request_id)
                next_request_id += 1
                if pong.get("type") != "pong":
                    raise RuntimeError(f"helper failed health ping: {pong}")
                print(f"completed {iteration + 1}/{args.iterations}", flush=True)

        session.send(request(next_request_id, "shutdown"))
        shutting_down = session.receive(next_request_id)
        if shutting_down.get("type") != "shutting_down":
            raise RuntimeError(f"helper rejected clean shutdown: {shutting_down}")
        return_code = session.process.wait(timeout=10)
        if return_code != 0:
            raise RuntimeError(f"helper exited with status {return_code}")
    except BaseException:
        session.stop()
        if session.stderr_lines:
            print("helper stderr:", file=sys.stderr)
            print("\n".join(session.stderr_lines[-20:]), file=sys.stderr)
        raise

    duration = time.monotonic() - started_at
    rss_summary = "unavailable"
    if initial_rss is not None and peak_rss is not None:
        rss_summary = (
            f"start={initial_rss / 1048576:.1f} MiB, "
            f"peak={peak_rss / 1048576:.1f} MiB"
        )
    print(
        f"PASS {args.engine_id}: {args.iterations} syntheses, "
        f"{total_frames} frames, {total_markers} markers, "
        f"{total_bytes / 1048576:.1f} MiB PCM in {duration:.1f}s; "
        f"launcher RSS {rss_summary}"
    )


if __name__ == "__main__":
    main()
