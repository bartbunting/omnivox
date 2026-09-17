#!/usr/bin/env python3
"""Check native bundled variants on two owned null-output speech workers."""
import argparse
import base64
import contextlib
import json
import os
from pathlib import Path
import subprocess
import time

from verify_voice_library_startup import server



def ordinary_speech(lane, voice):
    """Verify the normal palette path reports the selected physical voice."""
    dispatch = 1000 + lane.sequence
    lane.process.stdin.write(
        "c {[[logical_voice reading]]}\n"
        "q {Reading through the saved palette voice.}\n"
        f"emacsvox_marker_dispatch {dispatch}\n")
    lane.process.stdin.flush()
    deadline = time.monotonic() + 30
    realized = []
    while time.monotonic() < deadline:
        line = lane.lines.get(timeout=max(.01, deadline - time.monotonic()))
        assert line is not None, list(lane.errors)
        if line.startswith("__EMACSVOX_MARKER__ "):
            event = json.loads(base64.b64decode(line.split(" ", 1)[1]))
            if event.get("type") == "utterance_started":
                assert event["logical_voice_id"] == "reading", event
                realized.append(event["actual_voice"])
        if line.startswith(f"__EMACSVOX_TRACKED__ {dispatch} "):
            assert line.strip().endswith(" completed"), line
            assert realized and all(v == {"engine_id": "espeak", "voice_id": voice}
                                    for v in realized), (voice, realized)
            return
    raise AssertionError("ordinary palette speech timed out")

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", type=Path)
    parser.add_argument("--espeak-data", help="Native parent of a separate espeak-ng-data directory")
    args = parser.parse_args()
    program = args.server.resolve()
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith("OMNIVOX_") and key != "ESPEAK_NG_DATA"}
    forwarded = ["OMNIVOX_ESPEAK_VARIANTS", "OMNIVOX_AUDIO_OUTPUT", "ESPEAK_NG_DATA"]
    if args.espeak_data:
        environment["ESPEAK_NG_DATA"] = args.espeak_data
    environment["WSLENV"] = ":".join(
        [entry for entry in environment.get("WSLENV", "").split(":")
         if entry and entry.split("/", 1)[0] not in forwarded] + forwarded)
    catalogue = json.loads(subprocess.check_output(
        [str(program), "--list-espeak-variants"], env=environment, timeout=30))
    assert catalogue["schema_version"] == 1
    assert {"m1", "f1"} <= {v["id"] for v in catalogue["variants"]}
    base = next(v["id"]["voice_id"] for v in catalogue["bases"] if v["language"] == "en-us")
    first, second = base + "+m1", base + "+f1"
    environment["OMNIVOX_AUDIO_OUTPUT"] = "null"
    with contextlib.ExitStack() as stack:
        lanes = [stack.enter_context(server(program, ["--engine", "espeak", "--audio-output", "null"], environment))
                 for _ in range(2)]
        for lane in lanes:
            inventory = lane.control("inventory")
            engine = next(e for e in inventory["engines"] if e["id"] == "espeak")
            ids = {v["id"]["voice_id"] for v in engine["voices"]}
            assert base in ids and first not in ids and second not in ids
            assert "espeak_variants_v1" in lane.control("capabilities")["features"]
            assert {"m1", "f1"} <= {v["id"] for v in engine["espeak_variants"]}
            original_pid = lane.process.pid
            for voice in [first, second, base, first]:
                result = lane.control("preview", text="Testing the selected eSpeak voice.",
                                      selector=dict(kind="exact", engine_id="espeak", voice_id=voice))
                assert result["status"] == "completed" and result["realized"]["voice_id"] == voice, result
            policy = dict(preferred_engines=[], allow_same_language_on_requested_engine=False,
                          global_default=None, fallback_engines=[])
            for voice in [first, second, base, first]:
                definition = dict(id="reading", language="en-us", acss={},
                                  preferences=[dict(kind="exact", engine_id="espeak", voice_id=voice)])
                registration = lane.control("register_logical_voices", registry_generation=lane.sequence + 1,
                                            definitions=[definition], fallback_policy=policy)
                assert registration["registration"]["bindings"][0]["resolution"]["realized"]["voice_id"] == voice, registration
                ordinary_speech(lane, voice)
                result = lane.control("preview_voice", text="The palette uses this variant.",
                                      preferences=definition["preferences"], disabled_engine_ids=[], fallback_policy=policy)
                assert result["status"] == "completed" and result["realized"]["voice_id"] == voice, result
            missing = base + "+no-such-variant"
            result = lane.control("preview", text="This missing voice must fail.",
                                  selector=dict(kind="exact", engine_id="espeak", voice_id=missing))
            assert result["status"] != "completed" and result.get("realized") is None, result
            result = lane.control("preview_voice", text="The base voice supplies fallback.",
                                  preferences=[dict(kind="exact", engine_id="espeak", voice_id=missing),
                                               dict(kind="exact", engine_id="espeak", voice_id=base)],
                                  disabled_engine_ids=[], fallback_policy=policy)
            assert result.get("status") == "completed" and result["realized"]["voice_id"] == base, result
            lane.control("set_routing_policy", routing_policy_generation=1,
                         preferred_engine_ids=[], fallback_engine_ids=[], disabled_engine_ids=["espeak"])
            result = lane.control("preview", text="Disabled engines must stay disabled.",
                                  selector=dict(kind="exact", engine_id="espeak", voice_id=first))
            assert result["status"] != "completed" and result.get("realized") is None, result
            assert lane.process.pid == original_pid and lane.process.poll() is None
    print(f"Native variant catalogue ({len(catalogue['variants'])} variants), two-lane exact preview, "
          "base switching, palette registration and playback markers, engine disablement and fallback passed without restart (null output)")


if __name__ == "__main__":
    main()
