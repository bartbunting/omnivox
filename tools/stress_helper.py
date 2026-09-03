#!/usr/bin/env python3
"""Exercise one Omnivox capture helper repeatedly in a single process.

The test validates protocol ordering, PCM framing, marker bounds, health pings,
optional in-flight cancellation, and clean shutdown.  It uses only the Python
standard library so it can run from WSL against the Windows helper executables.
"""

import argparse
import base64
from datetime import datetime, timezone
import json
from pathlib import Path
import platform
import queue
import subprocess
import sys
import threading
import time

import process_metrics


PROTOCOL_VERSION = 5
SUPPORTED_PROTOCOL_VERSIONS = (5, 4, 3, 2, 1)
FRAME_TIMEOUT_SECONDS = 30.0
TEST_TEXTS = (
    "First sentence has several words. Second sentence checks completion!",
    "Unicode café and naïve words work here. Another sentence follows?",
    "A short clause, followed by another clause; then the sentence ends.",
)
RUTTS_TEST_TEXTS = (
    "Первое предложение содержит несколько слов. Второе проверяет завершение!",
    "Русский текст использует букву ё. Следующее предложение продолжается?",
    "Короткая фраза, затем другая фраза; и предложение заканчивается.",
)
CANCEL_PROBE_TEXT = " ".join(
    f"Cancellation probe sentence {number} should not reach the audio mixer."
    for number in range(1, 17)
)
RUTTS_CANCEL_PROBE_TEXT = " ".join(
    f"Фраза проверки отмены {number} не должна попасть в звуковой микшер."
    for number in range(1, 17)
)
DECTALK_NATIVE_INDEX = "[:index mark 12345]"


def exercise_text(engine_id, text, marker_capabilities):
    if engine_id == "dectalk" and marker_capabilities.get("native_index"):
        return f"{DECTALK_NATIVE_INDEX} {text}"
    return text


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

    def receive_any(self, timeout=FRAME_TIMEOUT_SECONDS):
        try:
            line = self.responses.get(timeout=timeout)
        except queue.Empty as error:
            raise RuntimeError("timed out waiting for a helper response") from error
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
        if response.get("type") == "error":
            raise RuntimeError(
                f"helper error {response.get('code')}: {response.get('message')}"
            )
        return response

    def receive(self, request_id, timeout=FRAME_TIMEOUT_SECONDS):
        response = self.receive_any(timeout)
        if response.get("request_id") != request_id:
            raise RuntimeError(f"unexpected request ID: {response}")
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


def synthesize(
    session,
    request_id,
    text,
    voice_id,
    iteration,
    acss_capabilities,
    marker_capabilities,
    progressive,
):
    settings = {
        "voice_id": voice_id,
        "rate": (0.35, 0.5, 0.7, 1.32)[iteration % 4],
        "pitch": (0.8, 1.0, 1.2)[iteration % 3],
        "volume": (0.35, 0.65, 1.0)[iteration % 3],
    }
    for dimension, values in (
        ("pitch_range", (0.1, 5.0 / 9.0, 0.9)),
        ("stress", (0.2, 5.0 / 9.0, 0.8)),
        ("richness", (0.3, 5.0 / 9.0, 0.7)),
    ):
        if acss_capabilities.get(dimension):
            settings[dimension] = values[iteration % len(values)]
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
    last_marker_offset = None
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
            if not isinstance(values, list) or not values:
                raise RuntimeError(f"helper emitted an invalid marker batch: {response}")
            if progressive:
                published_frames = audio_bytes // (channels * 2)
                for marker in values:
                    offset = marker.get("frame_offset")
                    if not isinstance(offset, int) or offset < published_frames:
                        raise RuntimeError(
                            "progressive helper emitted a marker behind published audio: "
                            f"{response}"
                        )
                    if last_marker_offset is not None and offset < last_marker_offset:
                        raise RuntimeError(
                            f"progressive helper markers are not monotonic: {response}"
                        )
                    last_marker_offset = offset
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
            required_kinds = {
                kind
                for kind in ("word", "sentence", "phoneme", "native_index")
                if marker_capabilities.get(kind)
            }
            if not required_kinds.issubset(kinds):
                missing_kinds = required_kinds - kinds
                raise RuntimeError(
                    f"helper omitted advertised markers {missing_kinds}: {kinds}"
                )
            anchor_support = marker_capabilities.get("requested_anchors", "none")
            if anchor_support != "none":
                expected_anchor = f"start-{iteration}"
                resolved = next(
                    (
                        marker
                        for marker in markers
                        if marker.get("kind") == "requested_anchor"
                        and marker.get("value") == expected_anchor
                    ),
                    None,
                )
                if resolved is None:
                    raise RuntimeError(
                        f"helper omitted requested anchor {expected_anchor}: {markers}"
                    )
                resolution = resolved.get("resolution", "exact")
                accepted_resolutions = {
                    "exact": {"exact"},
                    "word_boundary": {"exact", "word_boundary"},
                }.get(anchor_support, set())
                if resolution not in accepted_resolutions:
                    raise RuntimeError(
                        "helper returned an invalid requested-anchor resolution "
                        f"{resolution!r} for {anchor_support!r} support"
                    )
            return frame_count, len(markers), audio_bytes
        else:
            raise RuntimeError(f"unexpected synthesis response: {response}")


def cancel_synthesis(session, request_id, cancel_id, voice_id, text=CANCEL_PROBE_TEXT):
    session.send(
        request(
            request_id,
            "synthesize",
            text=text,
            settings={
                "voice_id": voice_id,
                "rate": 0.35,
                "pitch": 1.0,
                "volume": 1.0,
            },
            anchors=[],
        )
    )
    started = session.receive(request_id)
    if started.get("type") != "synthesis_started":
        raise RuntimeError(f"cancellation probe did not start synthesis: {started}")

    session.send(request(cancel_id, "cancel", target_request_id=request_id))
    acknowledged = False
    deadline = time.monotonic() + FRAME_TIMEOUT_SECONDS
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise RuntimeError("timed out waiting for cancellation to finish")
        response = session.receive_any(remaining)
        response_id = response.get("request_id")
        response_type = response.get("type")
        if response_id == cancel_id:
            if (
                acknowledged
                or response_type != "cancel_accepted"
                or response.get("target_request_id") != request_id
            ):
                raise RuntimeError(f"invalid cancellation acknowledgement: {response}")
            acknowledged = True
        elif response_id == request_id:
            if response_type in ("audio_chunk", "markers"):
                if acknowledged:
                    raise RuntimeError(
                        f"helper emitted stale synthesis output after cancellation: {response_type}"
                    )
            elif response_type == "synthesis_cancelled":
                if not acknowledged:
                    raise RuntimeError("synthesis ended before cancellation was acknowledged")
                return
            elif response_type == "synthesis_completed":
                raise RuntimeError("cancellation probe completed as successful synthesis")
            else:
                raise RuntimeError(f"unexpected cancellation response: {response}")
        else:
            raise RuntimeError(f"response belongs to an unknown request: {response}")


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("helper", help="path to an Omnivox helper executable")
    parser.add_argument("--engine-id", required=True, help="expected descriptor engine ID")
    parser.add_argument("--voice-id", help="voice to exercise; defaults to helper default")
    parser.add_argument("--iterations", type=int, default=100)
    parser.add_argument(
        "--require-streaming",
        action="store_true",
        help="fail unless protocol v5 reports progressive PCM delivery",
    )
    parser.add_argument(
        "--cancel-probe",
        action="store_true",
        help="cancel one long synthesis and verify the helper remains usable",
    )
    parser.add_argument(
        "--cancel-every",
        type=int,
        default=0,
        help="run an in-flight cancellation probe after every N syntheses",
    )
    parser.add_argument(
        "--health-every",
        type=int,
        default=25,
        help="send a ping after every N syntheses (default: 25; 0 disables)",
    )
    parser.add_argument(
        "--resource-sample-every",
        type=int,
        default=25,
        help="sample process resources every N syntheses (default: 25; 0 disables)",
    )
    parser.add_argument(
        "--require-acss",
        action="append",
        default=[],
        choices=(
            "rate",
            "average_pitch",
            "pitch_range",
            "stress",
            "richness",
            "volume",
        ),
        help="fail unless the descriptor advertises this ACSS dimension",
    )
    parser.add_argument(
        "--helper-arg",
        action="append",
        default=[],
        help="additional helper argument, such as an explicit native DLL path",
    )
    parser.add_argument(
        "--json-output",
        help="write a machine-readable soak report to this file, or '-' for stdout",
    )
    return parser.parse_args()


def main():
    args = parse_args()
    if args.iterations <= 0:
        raise SystemExit("--iterations must be positive")
    for field in ("cancel_every", "health_every", "resource_sample_every"):
        if getattr(args, field) < 0:
            raise SystemExit(f"--{field.replace('_', '-')} cannot be negative")
    command = [args.helper, *args.helper_arg]
    observer = process_metrics.ProcessObserver(command[0])
    session = HelperSession(command)
    started_at = time.monotonic()
    observer.bind(session.process.pid)
    resource_samples = []
    total_frames = 0
    total_markers = 0
    total_bytes = 0
    cancellation_probes = 0
    progress_stream = sys.stderr if args.json_output == "-" else sys.stdout

    def capture_resources(iteration, phase):
        metrics = observer.sample()
        if metrics is None:
            return
        resource_samples.append(
            {
                "iteration": iteration,
                "phase": phase,
                "elapsed_ms": (time.monotonic() - started_at) * 1000.0,
                **metrics,
            }
        )

    def health_ping(request_id):
        session.send(request(request_id, "ping"))
        pong = session.receive(request_id)
        if pong.get("type") != "pong":
            raise RuntimeError(f"helper failed health ping: {pong}")

    capture_resources(0, "started")
    try:
        session.send(
            request(
                1,
                "hello",
                supported_protocol_versions=list(SUPPORTED_PROTOCOL_VERSIONS),
            )
        )
        hello = session.receive(1)
        if (
            hello.get("type") != "hello"
            or hello.get("selected_protocol_version") != PROTOCOL_VERSION
        ):
            raise RuntimeError(f"helper failed protocol negotiation: {hello}")

        session.send(request(2, "describe"))
        described = session.receive(2)
        descriptor = described.get("descriptor", {})
        if described.get("type") != "descriptor" or descriptor.get("id") != args.engine_id:
            raise RuntimeError(f"helper returned the wrong descriptor: {described}")
        voice_id = args.voice_id or descriptor.get("default_voice_id")
        if not voice_id:
            raise RuntimeError("helper descriptor has no usable default voice")
        acss_capabilities = descriptor.get("capabilities", {}).get("acss", {})
        marker_capabilities = descriptor.get("capabilities", {}).get(
            "markers", {}
        )
        audio_output = descriptor.get("capabilities", {}).get("audio_output")
        if args.require_streaming and audio_output != "streaming_pcm":
            raise RuntimeError(
                f"helper did not advertise progressive PCM: {audio_output!r}"
            )
        missing_acss = [
            dimension
            for dimension in args.require_acss
            if not acss_capabilities.get(dimension)
        ]
        if missing_acss:
            raise RuntimeError(
                f"helper omitted required ACSS capabilities: {missing_acss}"
            )
        capture_resources(0, "ready")

        next_request_id = 3
        test_texts = RUTTS_TEST_TEXTS if args.engine_id == "rutts" else TEST_TEXTS
        cancel_text = (
            RUTTS_CANCEL_PROBE_TEXT
            if args.engine_id == "rutts"
            else CANCEL_PROBE_TEXT
        )
        for iteration in range(args.iterations):
            text = exercise_text(
                args.engine_id,
                test_texts[iteration % len(test_texts)],
                marker_capabilities,
            )
            frames, markers, byte_count = synthesize(
                session,
                next_request_id,
                text,
                voice_id,
                iteration,
                acss_capabilities,
                marker_capabilities,
                audio_output == "streaming_pcm",
            )
            next_request_id += 1
            total_frames += frames
            total_markers += markers
            total_bytes += byte_count

            completed_iterations = iteration + 1
            if args.cancel_every and completed_iterations % args.cancel_every == 0:
                cancel_synthesis(
                    session,
                    next_request_id,
                    next_request_id + 1,
                    voice_id,
                    cancel_text,
                )
                next_request_id += 2
                cancellation_probes += 1
                health_ping(next_request_id)
                next_request_id += 1
            if (
                args.resource_sample_every
                and completed_iterations % args.resource_sample_every == 0
            ):
                capture_resources(completed_iterations, "interval")
            if args.health_every and completed_iterations % args.health_every == 0:
                health_ping(next_request_id)
                next_request_id += 1
                print(
                    f"completed {completed_iterations}/{args.iterations}",
                    file=progress_stream,
                    flush=True,
                )

        if args.cancel_probe:
            cancel_synthesis(
                session,
                next_request_id,
                next_request_id + 1,
                voice_id,
                cancel_text,
            )
            next_request_id += 2
            cancellation_probes += 1
            health_ping(next_request_id)
            next_request_id += 1
            print(
                "completed in-flight cancellation probe",
                file=progress_stream,
                flush=True,
            )

        capture_resources(args.iterations, "before_shutdown")
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
    report = {
        "report_version": 1,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "host": {
            "platform": platform.platform(),
            "python": platform.python_version(),
        },
        "configuration": {
            "helper_name": process_metrics.executable_name(command[0]),
            "helper_argument_count": len(args.helper_arg),
            "engine_id": args.engine_id,
            "voice_id": voice_id,
            "iterations": args.iterations,
            "cancel_probe": args.cancel_probe,
            "cancel_every": args.cancel_every,
            "health_every": args.health_every,
            "resource_sample_every": args.resource_sample_every,
            "required_acss": args.require_acss,
            "require_streaming": args.require_streaming,
        },
        "helper": {
            "engine_id": descriptor.get("id"),
            "version": descriptor.get("version"),
            "voice_id": voice_id,
            "capabilities": descriptor.get("capabilities", {}),
        },
        "result": {
            "status": "completed",
            "duration_seconds": duration,
            "syntheses": args.iterations,
            "cancellation_probes": cancellation_probes,
            "frames": total_frames,
            "markers": total_markers,
            "pcm_bytes": total_bytes,
        },
        "resources": {
            **observer.description(),
            "samples": resource_samples,
            "summary": process_metrics.summarize_samples(resource_samples),
            "steady_state_summary": process_metrics.summarize_samples(
                [
                    sample
                    for sample in resource_samples
                    if sample["phase"] != "started"
                ]
            ),
        },
    }
    serialized = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.json_output == "-":
        print(serialized, end="")
        return

    print(
        f"PASS {args.engine_id}/{voice_id}: {args.iterations} syntheses, "
        f"{cancellation_probes} cancellations, {total_frames} frames, "
        f"{total_markers} markers, {total_bytes / 1048576:.1f} MiB PCM "
        f"in {duration:.1f}s; resources={observer.provider}"
    )
    if args.json_output:
        Path(args.json_output).write_text(serialized, encoding="utf-8")
        print(f"wrote raw helper soak report to {args.json_output}")


if __name__ == "__main__":
    main()
