import sys, json
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[4] / 'tools'))
from benchmark_server import ServerSession, configure_exact_voice, BENCHMARK_LOGICAL_VOICE_ID, MARKER_PREFIX, decode_record
class RecordingSession(ServerSession):
    def receive_line(self, deadline):
        observed, line = super().receive_line(deadline)
        if line.startswith(MARKER_PREFIX):
            events.append(decode_record(line[len(MARKER_PREFIX):]))
        return observed, line

events = []
server = sys.argv[1]
s = RecordingSession([server, '--audio-output', 'null'], 'espeak', 20)
try:
    caps, _ = s.negotiate(1)
    configure_exact_voice(s, caps, 'espeak', sys.argv[3] if len(sys.argv) > 3 else r'espeak:gmw\en-US', 2)
    s.send_line('tts_set_speech_rate 85')
    s.send_line('tts_set_punctuations some')
    for i in range(int(sys.argv[2]) if len(sys.argv) > 2 else 3):
        identifier = 100 + i
        s.send_timeline({'protocol_version': 3, 'generation': identifier,
            'dispatch_id': identifier, 'delivery_policy': 'ordered',
            'spans': [{'id': 1, 'text': ' dash ', 'logical_voice_id': BENCHMARK_LOGICAL_VOICE_ID},
                      {'id': 2, 'text': ' When both a personal copy and a shared copy have the same or similar name,',
                       'logical_voice_id': BENCHMARK_LOGICAL_VOICE_ID}], 'actions': []})
        result = s.wait_for_dispatches({identifier})[identifier]
        print(json.dumps({'dispatch_id': identifier, **result, 'last_markers': [e for e in events if e.get('dispatch_id') == identifier][-3:]}), flush=True)
finally:
    s.close()
    for line in s.stderr_lines:
        if 'out of order' in line: print(line, file=sys.stderr)
