#!/usr/bin/env python3
"""Check native bundled variants on two owned null-output speech workers."""
import argparse
import contextlib
import json
import os
from pathlib import Path
import subprocess

from verify_voice_library_startup import server


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", type=Path)
    args = parser.parse_args()
    program = args.server.resolve()
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith("OMNIVOX_") and key != "ESPEAK_NG_DATA"}
    forwarded = ["OMNIVOX_ESPEAK_VARIANTS", "OMNIVOX_AUDIO_OUTPUT"]
    environment["WSLENV"] = ":".join(
        [entry for entry in environment.get("WSLENV", "").split(":")
         if entry and entry.split("/", 1)[0] not in forwarded] + forwarded)
    catalogue = json.loads(subprocess.check_output(
        [str(program), "--list-espeak-variants"], env=environment, timeout=30))
    assert catalogue["schema_version"] == 1
    assert {"m1", "f1"} <= {v["id"] for v in catalogue["variants"]}
    base = next(v["id"]["voice_id"] for v in catalogue["bases"] if v["language"] == "en-us")
    enabled, disabled = base + "+m1", base + "+f1"
    environment["OMNIVOX_ESPEAK_VARIANTS"] = json.dumps([
        dict(base_voice_id=base, variant_id="m1", enabled=True),
        dict(base_voice_id=base, variant_id="f1", enabled=False)])
    environment["OMNIVOX_AUDIO_OUTPUT"] = "null"
    with contextlib.ExitStack() as stack:
        lanes = [stack.enter_context(server(program, ["--engine", "espeak", "--audio-output", "null"], environment))
                 for _ in range(2)]
        for lane in lanes:
            inventory = lane.control("inventory")
            engine = next(e for e in inventory["engines"] if e["id"] == "espeak")
            ids = {v["id"]["voice_id"] for v in engine["voices"]}
            assert enabled in ids and disabled not in ids and base in ids
            status = lane.control("voice_library_status_v1")
            eligible = {v["voice_id"] for v in status["eligible_voices"] if v["engine_id"] == "espeak"}
            assert enabled in eligible and disabled not in eligible
            for voice in [enabled, base, enabled]:
                result = lane.control("preview", text="Testing the selected eSpeak voice.",
                                      selector=dict(kind="exact", engine_id="espeak", voice_id=voice))
                assert result["status"] == "completed" and result["realized"]["voice_id"] == voice, result
            result = lane.control("preview", text="This disabled voice must fail.",
                                  selector=dict(kind="exact", engine_id="espeak", voice_id=disabled))
            assert result["status"] != "completed" and result.get("realized") is None, result
            result = lane.control("preview_voice", text="The base voice supplies fallback.",
                                  preferences=[dict(kind="exact", engine_id="espeak", voice_id=disabled),
                                               dict(kind="exact", engine_id="espeak", voice_id=base)],
                                  disabled_engine_ids=[],
                                  fallback_policy=dict(preferred_engines=[],
                                                       allow_same_language_on_requested_engine=False,
                                                       global_default=None, fallback_engines=[]))
            assert result.get("status") == "completed" and result["realized"]["voice_id"] == base, result
    print(f"Native variant catalogue ({len(catalogue['variants'])} variants), two-lane exact preview, "
          "base switching, eligibility, disabled rejection and fallback passed with null output")


if __name__ == "__main__":
    main()
