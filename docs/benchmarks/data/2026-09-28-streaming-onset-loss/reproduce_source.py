#!/usr/bin/env python3
"""Reproduce the released converter's onset discard in an isolated archive."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib

import numpy as np


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    fixture = Path(__file__).resolve().parent
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    source = output / "source"
    source.mkdir()
    commit = subprocess.check_output(
        ["git", "rev-parse", "v1.14.0^{commit}"], cwd=args.repo, text=True
    ).strip()
    with tempfile.TemporaryFile() as archive:
        subprocess.run(["git", "archive", commit], cwd=args.repo, stdout=archive, check=True)
        archive.seek(0)
        subprocess.run(["tar", "-xf", "-", "-C", str(source)], stdin=archive, check=True)
    examples = source / "omnivox-audio/examples"
    examples.mkdir(exist_ok=True)
    shutil.copyfile(fixture / "onset_probe.rs", examples / "onset_probe.rs")
    toolchain = tomllib.loads((source / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    results = {"commit": commit, "toolchain": toolchain, "cases": []}
    command = ["cargo", "+" + toolchain, "run", "--locked", "-p", "omnivox-audio",
               "--example", "onset_probe", "--"]
    for name, rate in [("chirp-22050-1", 22050), ("chirp-16000-1", 16000),
                       ("chirp-22050-1-lead", 22050)]:
        prefix = output / name
        with (output / (name + ".log")).open("w") as log:
            subprocess.run(command + [str(fixture / (name + ".pcm")), str(rate), "1", str(prefix)],
                           cwd=source, stdout=log, stderr=subprocess.STDOUT, check=True)
        buffered = np.fromfile(str(prefix) + ".buffered.f32", "<f4").reshape(-1, 2)
        progressive = np.fromfile(str(prefix) + ".progressive.f32", "<f4").reshape(-1, 2)
        skip = int(128 * 44100 / rate)
        assert np.array_equal(buffered[skip:], progressive[:len(buffered) - skip])
        assert not np.array_equal(buffered[:100], progressive[:100])
        results["cases"].append({"input": name, "discarded_frames": skip,
                                 "shifted_overlap_exact": True})
    # Diagnostic mutation of the isolated archive only; never the supplied repo.
    converter = source / "omnivox-audio/src/progressive_pcm.rs"
    old = converter.read_text()
    needle = "let output_delay_remaining = resampler.as_ref().map_or(0, Resampler::output_delay);"
    assert old.count(needle) == 1
    converter.write_text(old.replace(needle, "let output_delay_remaining = 0;"))
    prefix = output / "control"
    with (output / "control.log").open("w") as log:
        subprocess.run(command + [str(fixture / "chirp-22050-1.pcm"), "22050", "1", str(prefix)],
                       cwd=source, stdout=log, stderr=subprocess.STDOUT, check=True)
    buffered = np.fromfile(str(prefix) + ".buffered.f32", "<f4")
    progressive = np.fromfile(str(prefix) + ".progressive.f32", "<f4")
    assert np.array_equal(buffered, progressive[:len(buffered)])
    results["zero_discard_control_prefix_exact"] = True
    results["probe_sha256"] = hashlib.sha256((fixture / "onset_probe.rs").read_bytes()).hexdigest()
    (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
