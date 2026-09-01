#!/usr/bin/env python3
"""Measure exact Omnivox engines across normalized host speech rates."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import statistics
import struct
import subprocess
import sys
import tempfile
from typing import Sequence


REPORT_VERSION = 1
DEFAULT_TEXT = (
    "Clear speech helps people review documents, navigate software, compare "
    "voices, and understand information comfortably during a normal working "
    "day without unnecessary effort."
)


class AuditError(RuntimeError):
    """A rate-audit input or synthesis failed validation."""


@dataclass(frozen=True)
class EngineTarget:
    engine: str
    voice: str

    @property
    def label(self) -> str:
        return self.engine if not self.voice else f"{self.engine}={self.voice}"


def parse_target(value: str) -> EngineTarget:
    engine, separator, voice = value.partition("=")
    if not engine or not re.fullmatch(r"[A-Za-z0-9_-]+", engine):
        raise argparse.ArgumentTypeError(
            "engine targets must be ENGINE or ENGINE=VOICE with a simple engine ID"
        )
    return EngineTarget(engine=engine, voice=voice if separator else "")


def parse_rates(value: str) -> list[float]:
    try:
        rates = [float(item.strip()) for item in value.split(",") if item.strip()]
    except ValueError as error:
        raise argparse.ArgumentTypeError("rates must be comma-separated numbers") from error
    if not rates or any(not 0.0 <= rate <= 2.0 for rate in rates):
        raise argparse.ArgumentTypeError("rates must contain values from 0.0 through 2.0")
    if len(set(rates)) != len(rates):
        raise argparse.ArgumentTypeError("rates must not contain duplicates")
    return rates


def count_words(text: str) -> int:
    return len(re.findall(r"\b[^\W_]+(?:[-'][^\W_]+)*\b", text, re.UNICODE))


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def wav_duration(path: Path) -> float:
    """Return duration from a bounded RIFF/WAVE PCM or IEEE-float file."""
    size = path.stat().st_size
    if size < 12 or size > 4 * 1024 * 1024 * 1024:
        raise AuditError(f"invalid WAV size for {path.name}: {size} bytes")

    byte_rate = None
    data_bytes = 0
    with path.open("rb") as source:
        if source.read(4) != b"RIFF":
            raise AuditError(f"{path.name} is not a RIFF file")
        source.read(4)
        if source.read(4) != b"WAVE":
            raise AuditError(f"{path.name} is not a WAVE file")

        while source.tell() + 8 <= size:
            chunk_id = source.read(4)
            chunk_size_raw = source.read(4)
            if len(chunk_id) != 4 or len(chunk_size_raw) != 4:
                break
            chunk_size = struct.unpack("<I", chunk_size_raw)[0]
            if chunk_size > size - source.tell():
                raise AuditError(f"truncated {chunk_id!r} chunk in {path.name}")
            if chunk_id == b"fmt ":
                payload = source.read(chunk_size)
                if len(payload) < 16:
                    raise AuditError(f"short fmt chunk in {path.name}")
                audio_format, channels, sample_rate, parsed_byte_rate = struct.unpack(
                    "<HHII", payload[:12]
                )
                if audio_format not in (1, 3) or not channels or not sample_rate:
                    raise AuditError(f"unsupported WAV format in {path.name}")
                byte_rate = parsed_byte_rate
            elif chunk_id == b"data":
                data_bytes += chunk_size
                source.seek(chunk_size, os.SEEK_CUR)
            else:
                source.seek(chunk_size, os.SEEK_CUR)
            if chunk_size % 2:
                source.seek(1, os.SEEK_CUR)

    if not byte_rate or not data_bytes:
        raise AuditError(f"{path.name} has no usable fmt/data chunks")
    return data_bytes / byte_rate


def output_argument(path: Path, windows_output_paths: bool) -> str:
    if not windows_output_paths:
        return str(path)
    try:
        conversion = subprocess.run(
            ["wslpath", "-w", str(path)],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise AuditError(f"could not translate WSL output path {path}: {error}") from error
    return conversion.stdout.strip()


def run_version(command_prefix: Sequence[str], timeout: float) -> str:
    try:
        result = subprocess.run(
            [*command_prefix, "--version"],
            check=True,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise AuditError(f"could not query Omnivox version: {error}") from error
    return result.stdout.strip()


def run_target(
    command_prefix: Sequence[str],
    target: EngineTarget,
    rates: Sequence[float],
    repetitions: int,
    text: str,
    word_count: int,
    directory: Path,
    timeout: float,
    piper_model: str | None,
    windows_output_paths: bool,
) -> dict[str, object]:
    samples: list[dict[str, object]] = []
    safe_voice = re.sub(r"[^A-Za-z0-9_.-]+", "_", target.voice)[:48]
    safe_target = target.engine + (f"-{safe_voice}" if safe_voice else "")

    for rate in rates:
        for repetition in range(1, repetitions + 1):
            output = directory / f"{safe_target}-{rate:g}-{repetition}.wav"
            raw = Path(str(output).replace(".wav", "_raw.wav"))
            if output.exists() or raw.exists():
                raise AuditError(
                    f"refusing to overwrite existing WAV for {target.label} "
                    f"rate {rate:g}, repetition {repetition}"
                )
            command = [
                *command_prefix,
                "--engine",
                target.engine,
                "--rate",
                f"{rate:.9g}",
                "--pitch",
                "1.0",
                "--voice-volume",
                "1.0",
            ]
            if piper_model:
                command.extend(["--piper-model", piper_model])
            command.extend(
                [
                    "--dump-wav",
                    target.voice,
                    output_argument(output, windows_output_paths),
                    text,
                ]
            )
            try:
                result = subprocess.run(
                    command,
                    check=False,
                    capture_output=True,
                    text=True,
                    timeout=timeout,
                )
            except (OSError, subprocess.SubprocessError) as error:
                raise AuditError(
                    f"{target.label} rate {rate:g} could not run: {error}"
                ) from error
            if result.returncode:
                detail = result.stderr.strip() or result.stdout.strip() or "no diagnostic output"
                raise AuditError(
                    f"{target.label} rate {rate:g} failed with exit "
                    f"{result.returncode}: {detail}"
                )
            if not raw.is_file():
                raise AuditError(
                    f"{target.label} rate {rate:g} did not create {raw.name}"
                )
            if not output.is_file():
                raise AuditError(
                    f"{target.label} rate {rate:g} did not create {output.name}"
                )
            raw_duration = wav_duration(raw)
            pipeline_duration = wav_duration(output)
            samples.append(
                {
                    "rate": rate,
                    "repetition": repetition,
                    "raw_duration_seconds": round(raw_duration, 6),
                    "pipeline_duration_seconds": round(pipeline_duration, 6),
                    "words_per_minute": round(
                        word_count * 60.0 / pipeline_duration, 3
                    ),
                }
            )

    summary = []
    for rate in rates:
        selected = [sample for sample in samples if sample["rate"] == rate]
        raw_durations = [float(sample["raw_duration_seconds"]) for sample in selected]
        pipeline_durations = [
            float(sample["pipeline_duration_seconds"]) for sample in selected
        ]
        wpms = [float(sample["words_per_minute"]) for sample in selected]
        summary.append(
            {
                "rate": rate,
                "median_raw_duration_seconds": round(
                    statistics.median(raw_durations), 6
                ),
                "median_pipeline_duration_seconds": round(
                    statistics.median(pipeline_durations), 6
                ),
                "median_words_per_minute": round(statistics.median(wpms), 3),
                "minimum_pipeline_duration_seconds": round(min(pipeline_durations), 6),
                "maximum_pipeline_duration_seconds": round(max(pipeline_durations), 6),
            }
        )

    return {
        "engine": target.engine,
        "voice": target.voice or None,
        "samples": samples,
        "summary": summary,
    }


def write_report(path: Path, report: dict[str, object]) -> None:
    if path.exists():
        raise AuditError(f"refusing to overwrite existing report: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    temporary.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    temporary.replace(path)


def print_summary(results: Sequence[dict[str, object]]) -> None:
    print("engine\tvoice\trate\tpipeline_seconds\traw_seconds\twords_per_minute")
    for result in results:
        voice = result["voice"] or "default"
        for summary in result["summary"]:
            print(
                f"{result['engine']}\t{voice}\t{summary['rate']:g}\t"
                f"{summary['median_pipeline_duration_seconds']:.6f}\t"
                f"{summary['median_raw_duration_seconds']:.6f}\t"
                f"{summary['median_words_per_minute']:.3f}"
            )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("program", help="Omnivox executable or launcher to audit")
    parser.add_argument(
        "--target",
        action="append",
        type=parse_target,
        dest="targets",
        required=True,
        help="exact ENGINE or ENGINE=VOICE target; repeat for more engines",
    )
    parser.add_argument(
        "--rates", type=parse_rates, default=parse_rates("0.3,0.4,0.5,0.6,0.8")
    )
    parser.add_argument("--repetitions", type=int, default=1)
    parser.add_argument("--text", default=DEFAULT_TEXT)
    parser.add_argument("--word-count", type=int)
    parser.add_argument("--piper-model")
    parser.add_argument("--timeout", type=float, default=120.0)
    parser.add_argument("--work-dir", type=Path)
    parser.add_argument(
        "--windows-output-paths",
        action="store_true",
        help="translate output paths with wslpath for a Windows executable under WSL",
    )
    parser.add_argument("--json-output", type=Path)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    targets = args.targets
    if args.repetitions < 1 or args.repetitions > 100:
        raise AuditError("repetitions must be from 1 through 100")
    if args.timeout <= 0:
        raise AuditError("timeout must be positive")
    if args.json_output and args.json_output.exists():
        raise AuditError(f"refusing to overwrite existing report: {args.json_output}")
    word_count = args.word_count if args.word_count is not None else count_words(args.text)
    if word_count <= 0:
        raise AuditError("text must contain words or --word-count must be positive")

    command_prefix = [args.program]
    version = run_version(command_prefix, args.timeout)
    program_path = Path(args.program)
    program_hash = file_sha256(program_path) if program_path.is_file() else None

    owned_temporary = None
    if args.work_dir:
        directory = args.work_dir.resolve()
        directory.mkdir(parents=True, exist_ok=True)
    else:
        owned_temporary = tempfile.TemporaryDirectory(prefix="omnivox-rate-audit-")
        directory = Path(owned_temporary.name)

    try:
        results = [
            run_target(
                command_prefix,
                target,
                args.rates,
                args.repetitions,
                args.text,
                word_count,
                directory,
                args.timeout,
                args.piper_model,
                args.windows_output_paths,
            )
            for target in targets
        ]
        report = {
            "schema_version": REPORT_VERSION,
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "program": {
                "name": program_path.name,
                "sha256": program_hash,
                "version": version,
            },
            "host": {
                "platform": platform.platform(),
                "python": platform.python_version(),
            },
            "corpus": {
                "sha256": hashlib.sha256(args.text.encode("utf-8")).hexdigest(),
                "utf8_bytes": len(args.text.encode("utf-8")),
                "word_count": word_count,
            },
            "repetitions": args.repetitions,
            "results": results,
        }
        print_summary(results)
        if args.json_output:
            write_report(args.json_output, report)
            print(f"report\t{args.json_output}")
        if args.work_dir:
            print(f"wav_directory\t{directory}")
        return 0
    finally:
        if owned_temporary:
            owned_temporary.cleanup()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except AuditError as error:
        print(f"rate audit failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
