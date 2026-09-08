#!/usr/bin/env python3
"""Per-choice acceptance on real isolated speaker/notification workers.

Uses make dev's staged payload and a null audio sink. The inherited remote
acceptance suite also checks old commands, framing and disconnect behaviour.
"""
from __future__ import annotations

import base64
import copy
import json
import unittest

import test_remote_service as remote


FIXTURE = json.loads((remote.ROOT / "docs/protocol-fixtures/voice-choice-tuning.json").read_text())
BUNDLE = {"voice_choice_tuning_v1", "presentation_timeline_v4", "playback_marker_events_v3"}


def encode(value):
    return base64.b64encode(json.dumps(value, ensure_ascii=False).encode()).decode()


def control(peer, payload):
    peer.send("omnivox_control " + encode(payload) + "\n")
    result = json.loads(base64.b64decode(peer.until("__OMNIVOX_CONTROL__ ").split(" ", 1)[1]))
    if result.get("request_id") != payload["request_id"]:
        raise AssertionError(result)
    return result


class VoiceChoiceRemoteTests(remote.RemoteServiceTests):
    def registration(self, peer, row_id):
        inventory = peer.control("inventory", 700)
        engine = next(engine for engine in inventory["engines"] if engine["id"] == "espeak")
        physical = engine["voices"][0]["id"]
        registration = copy.deepcopy(FIXTURE["messages"]["registration"])
        voice = registration["definitions"][0]["definition"]
        voice["choices"] = [
            {"id": "unavailable", "selector": {"kind": "exact", "engine_id": "absent-choice-test", "voice_id": "absent"}, "adjustments": {}},
            {"id": row_id, "selector": {"kind": "exact", **physical}, "adjustments": {"average_pitch": {"op": "set", "value": 0.6}}},
            {"id": row_id + "-duplicate", "selector": {"kind": "exact", **physical}, "adjustments": {"average_pitch": {"op": "default"}}},
        ]
        self.assertEqual(control(peer, registration)["type"], "logical_voices_registered_v2")
        return registration

    def speak(self, peer, dispatch, generation, expected_row, multipart=False, registry_generation=41):
        timeline = copy.deepcopy(FIXTURE["messages"]["timeline"])
        timeline.update(dispatch_id=dispatch, generation=generation, registry_generation=registry_generation)
        begin = len(peer.received)
        encoded = encode(timeline)
        if multipart:
            split = len(encoded) // 8 * 4
            size = len(base64.b64decode(encoded))
            for index, fragment in enumerate((encoded[:split], encoded[split:])):
                peer.send(f"emacsvox_timeline_part 4 {generation} {dispatch} {index} 2 {size} {fragment}\n")
        else:
            peer.send("emacsvox_timeline " + encoded + "\n")
        terminal = peer.until(f"__EMACSVOX_TRACKED__ {dispatch} ")
        events = [json.loads(base64.b64decode(line.split(" ", 1)[1]))
                  for line in peer.received[begin:] if line.startswith("__EMACSVOX_MARKER__ ")]
        if expected_row is None:
            self.assertTrue(terminal.endswith(" failed"), terminal)
            self.assertEqual(events, [])
            return
        self.assertTrue(terminal.endswith(" completed"), terminal)
        receipts = []
        for index, event in enumerate(events):
            self.assertEqual(event["protocol_version"], 3)
            if event["type"] == "voice_choice_applied":
                self.assertEqual(events[index - 1]["type"], "utterance_started")
                self.assertEqual(event["sequence"], events[index - 1]["sequence"] + 1)
                self.assertEqual(event["choice"]["choice_id"], expected_row)
                self.assertEqual(event["registry_generation"], registry_generation)
                receipts.append(event["span_id"])
        self.assertEqual(receipts, [1, 3])
        self.assertEqual(sum(event["type"] == "utterance_started" for event in events), 4)

    def test_choice_registrations_and_partial_assemblies_are_lane_and_connection_owned(self):
        speaker = self.connect()
        notification = self.connect("notification")
        for peer in (speaker, notification):
            features = set(peer.control("capabilities")["features"])
            self.assertTrue(BUNDLE.issubset(features))
        speaker_registration = self.registration(speaker, "speaker-fallback")
        self.registration(notification, "notification-fallback")
        self.speak(speaker, 801, 8, "speaker-fallback", multipart=True)
        self.speak(notification, 801, 8, "notification-fallback")

        # Duplicate physical selectors retain the requested original row.
        preview = copy.deepcopy(FIXTURE["messages"]["preview"])
        voice = speaker_registration["definitions"][0]["definition"]
        preview["voice"] = {key: voice[key] for key in ("language", "shared", "choices")}
        preview.pop("expected_base_rate")
        preview["selection"] = {"mode": "choice", "choice_id": "speaker-fallback-duplicate"}
        result = control(speaker, preview)
        self.assertEqual(result["type"], "preview_voice_completed_v2")
        self.assertEqual(result["status"], "completed")
        self.assertEqual(result["last_started"]["choice_id"], "speaker-fallback-duplicate")
        self.assertEqual(result["last_started"]["reason"], {"reason": "explicit_alternative", "preference_index": 2})
        self.assertTrue(result["accepted_audio"][0]["playback_started"])

        # Disconnect during a multipart request. Neither definitions nor a
        # prefix of its old document may be reused by the replacement worker.
        encoded = encode(FIXTURE["messages"]["timeline"])
        fragment = encoded[:len(encoded) // 8 * 4]
        speaker.send(f"emacsvox_timeline_part 4 9 899 0 2 {len(base64.b64decode(encoded))} {fragment}\n")
        speaker.close()
        replacement = self.reconnect()
        replacement.control("capabilities")
        self.speak(replacement, 802, 1, None)
        self.speak(notification, 802, 9, "notification-fallback", multipart=True)
        self.registration(replacement, "reconnected-fallback")
        self.speak(replacement, 803, 2, "reconnected-fallback")
        self.assertFalse(any("__EMACSVOX_TRACKED__ 899 " in line for line in replacement.received))

        # A newer old registration replaces the mixed registry completely;
        # timeline 4 cannot quietly reuse the previous layered definition.
        legacy = copy.deepcopy(speaker_registration)
        legacy.update(type="register_logical_voices", registry_generation=42)
        legacy["definitions"] = [{"id": "bolden", "language": None,
                                   "preferences": [voice["choices"][1]["selector"]],
                                   "acss": {}, "effects": {}}]
        self.assertEqual(control(replacement, legacy)["type"], "logical_voices_registered")
        self.speak(replacement, 804, 3, None, registry_generation=42)
        replacement.send("q Legacy remains usable.\nemacsvox_marker_dispatch 805\n")
        self.assertTrue(replacement.until("__EMACSVOX_TRACKED__ 805 ").endswith(" completed"))
        self.speak(notification, 803, 10, "notification-fallback")


if __name__ == "__main__":
    unittest.main()
