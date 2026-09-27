"""Local protocol fixture: unknown identity, literal args and short deterministic PCM.

Tests prepend the native Python interpreter path and launch the resulting executable
from a path containing spaces. This fixture is never part of a release payload.
"""
import argparse
import base64
import json
import sys
import time

parser = argparse.ArgumentParser()
parser.add_argument("--descriptor", required=True)
parser.add_argument("--record", required=True)
parser.add_argument("--tag", default="")
parser.add_argument("--hang", action="store_true")
args = parser.parse_args()
with open(args.record, "a", encoding="utf-8") as record:
    record.write(json.dumps(sys.argv[1:]) + "\n")
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
            b"\x00\x00\x00\x01\x00\xff\x00\x00").decode("ascii")))
        reply("synthesis_completed", frame_count=4)
    elif kind == "cancel":
        reply("cancel_accepted", target_request_id=request["target_request_id"])
    elif kind == "ping":
        reply("pong")
    elif kind == "shutdown":
        reply("shutting_down")
        break
