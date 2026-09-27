"""Local protocol fixture: unknown identity, literal args and short deterministic PCM.

Tests prepend the native Python interpreter path and launch the resulting executable
from a path containing spaces. This fixture is never part of a release payload.
"""
import argparse
import base64
import json
import math
import os
import sys
import struct
import time

parser = argparse.ArgumentParser()
parser.add_argument("--descriptor", required=True)
parser.add_argument("--record", required=True)
parser.add_argument("--tag", default="")
parser.add_argument("--empty", default="")
parser.add_argument("--hang", action="store_true")
parser.add_argument("--record-environment")
args = parser.parse_args()
with open(args.record, "a", encoding="utf-8") as record:
    record.write(json.dumps(sys.argv[1:]) + "\n")
if args.record_environment:
    with open(args.record_environment, "a", encoding="utf-8") as record:
        record.write(json.dumps(dict(pid=os.getpid(), value=os.environ.get("OMNIVOX_FIXTURE_PRIVATE"))) + "\n")
if args.hang:
    time.sleep(300)
with open(args.descriptor, encoding="utf-8") as source:
    descriptor = json.load(source)

for line in sys.stdin:
    request = json.loads(line)
    def reply(kind, **fields):
        print(json.dumps(dict(protocol_version=1, request_id=request["request_id"],
                              type=kind, **fields)), flush=True)
    kind = request["type"]
    if kind == "hello":
        reply("hello", selected_protocol_version=1,
              helper_name="Framework fixture", helper_version="1")
    elif kind == "describe":
        reply("descriptor", descriptor=descriptor)
    elif kind == "synthesize":
        reply("synthesis_started", format=dict(sample_rate=22050, channels=1,
                                               sample_format="pcm_s16_le"),
              actual_voice_id=descriptor["default_voice_id"])
        reply("audio_chunk", chunk=dict(sequence=0, data_base64=base64.b64encode(
            b"".join(struct.pack("<h", int(8192 * math.sin(2 * math.pi * 440 * frame / 22050)))
                     for frame in range(2200))).decode("ascii")))
        reply("synthesis_completed", frame_count=2200)
    elif kind == "cancel":
        reply("cancel_accepted", target_request_id=request["target_request_id"])
    elif kind == "ping":
        reply("pong")
    elif kind == "shutdown":
        reply("shutting_down")
        break
