#!/usr/bin/env python3
# Copyright (C) 2026 Bart Bunting
# SPDX-License-Identifier: GPL-2.0-or-later
"""Silent direct-wire Eloquence helper-6 acceptance; never installs a runtime."""
import argparse, base64, copy, hashlib, json, pathlib, queue, subprocess, threading, time

class Session:
    def __init__(self, program, dll, version=6):
        self.process = subprocess.Popen([str(program), dll], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
        self.frames = queue.Queue()
        self.diagnostics = []
        self.version, self.sequence = version, 0
        self.evidence = []
        def read():
            for line in self.process.stdout:
                self.frames.put(json.loads(line))
            self.frames.put(None)
        def logs():
            for line in self.process.stderr:
                self.diagnostics.append(line)
                self.diagnostics = self.diagnostics[-20:]
        threading.Thread(target=read, daemon=True).start()
        threading.Thread(target=logs, daemon=True).start()
        result = self.request('hello', supported_protocol_versions=[version])[-1]
        assert result['type'] == 'hello' and result['selected_protocol_version'] == version, result
    def send(self, kind, **fields):
        self.sequence += 1
        message = dict(protocol_version=self.version, request_id=self.sequence, type=kind, **fields)
        self.process.stdin.write(json.dumps(message, separators=(',', ':')) + '\n')
        self.process.stdin.flush()
        return message
    def receive(self, message, terminal=None):
        frames = []
        while True:
            frame = self.frames.get(timeout=15)
            assert frame is not None, ''.join(self.diagnostics)
            assert frame['request_id'] == message['request_id'], frame
            frames.append(frame)
            if terminal is None or frame['type'] in terminal:
                break
        for frame in frames:
            if frame['type'] in ('engine_parameters_v1', 'voice_parameters_explained_v1', 'synthesis_started'):
                self.evidence.append({'request': message, 'response': frame})
        return frames
    def request(self, kind, **fields):
        return self.receive(self.send(kind, **fields), {'synthesis_completed', 'synthesis_cancelled', 'error'} if kind == 'synthesize' else None)
    def close(self):
        try:
            self.request('shutdown')
            self.process.stdin.close()
            self.process.wait(timeout=10)
        finally:
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=5)
    def catalogue(self, voice=None):
        deadline = time.monotonic() + 12
        while True:
            frame = self.request('get_engine_parameters_v1', engine_id='eloquence', voice_id=voice, cursor=None, expected_catalogue_revision=None)[0]
            assert frame['type'] == 'engine_parameters_v1', frame
            if frame['result']['status'] != 'busy':
                assert frame['result']['status'] == 'ready', frame
                return frame['result']
            assert time.monotonic() < deadline, frame
            time.sleep(0.05)

SETTINGS = dict(voice_id='v1', rate=0.5, pitch=1.0, pitch_range=None, stress=None, richness=None, volume=1.0)

def patch(identity, operations=None, context=None, policy='require'):
    return dict(native=dict(engine_id='eloquence', schema_id=identity['schema_id'], parameters=operations or {}), context_dimensions=context or [], expected_identity=copy.deepcopy(identity), unavailable_policy=policy)

def synth(session, settings, parameters, text='Native parameter test.'):
    fields = dict(text=text, settings=settings, anchors=[])
    if session.version == 6:
        fields['voice_parameters'] = parameters
    frames = session.request('synthesize', **fields)
    assert frames[-1]['type'] == 'synthesis_completed', frames[-1]
    assert frames[0]['type'] == 'synthesis_started', [f['type'] for f in frames]
    pcm = b''.join(base64.b64decode(f['chunk']['data_base64']) for f in frames if f['type'] == 'audio_chunk')
    assert pcm and len(pcm) // 2 == frames[-1]['frame_count']
    return frames[0], len(pcm) // 2

def streaming_cases(session, identity):
    frames = session.request('synthesize', text='Native marker test.', settings=SETTINGS,
        anchors=[dict(id='start', text_offset=0, affinity='before'), dict(id='end', text_offset=19, affinity='after')],
        voice_parameters=patch(identity, {'head_size': dict(op='set', value=70)}))
    assert frames[0]['type'] == 'synthesis_started' and frames[-1]['type'] == 'synthesis_completed', frames[-1]
    markers = [m for f in frames if f['type'] == 'markers' for m in f['markers']]
    anchors = [m for m in markers if m['kind'] == 'requested_anchor']
    assert {m['value'] for m in anchors} == {'start', 'end'}, anchors
    request = session.send('synthesize', text='Cancellation sample. ' * 500, settings=SETTINGS, anchors=[],
        voice_parameters=patch(identity, {'speed': dict(op='set', value=50), 'head_size': dict(op='set', value=70)}))
    first = session.frames.get(timeout=15)
    assert first['request_id'] == request['request_id'] and first['type'] == 'synthesis_started', first
    cancel = session.send('cancel', target_request_id=request['request_id'])
    accepted = False
    kinds = []
    while True:
        frame = session.frames.get(timeout=15)
        kind = frame['type']
        if frame['request_id'] == cancel['request_id']:
            assert kind == 'cancel_accepted', frame
            accepted = True
        else:
            assert frame['request_id'] == request['request_id'], frame
            if accepted:
                assert kind == 'synthesis_cancelled', kind
            assert kind not in ('error', 'synthesis_completed'), frame
            if kind == 'synthesis_cancelled':
                assert accepted
                kinds.append(kind)
                break
        if kind != 'audio_chunk':
            kinds.append(kind)
    ordinary = synth(session, SETTINGS, None)
    started, _ = synth(session, SETTINGS, patch(identity))
    result = session.request('explain_voice_parameters_v1', source=dict(mode='applied',
        plan_id=started['native_application']['plan_id']))[0]['result']
    assert all(row['read_back'] for row in result['parameters'])
    return dict(anchors=anchors, cancellation_frames=kinds, followup_applied=result,
        ordinary_followup_frames=ordinary[1])

def run(program, dll):
    s = Session(program, dll)
    cases = 0
    try:
        catalogue = s.catalogue()
        identity = catalogue['identity']
        assert len(catalogue['parameters']) == 8 and catalogue['next_cursor'] is None
        assert all(p['default'] == dict(source='unknown', value=None, reset_supported=True) for p in catalogue['parameters'])
        assert s.catalogue('v1')['identity'] == identity
        cases += 1
        # Read-only planning preserves ordinary speech and leaves unknown preset values unknown.
        started, baseline = synth(s, SETTINGS, None)
        assert started['native_application'] is None
        native = patch(identity, {'head_size': {'op': 'set', 'value': 70}, 'speed': {'op': 'set', 'value': 80}, 'volume': {'op': 'set', 'value': 10}}, ['richness'])
        settings = dict(SETTINGS, richness=0.5)
        planned = s.request('explain_voice_parameters_v1', source=dict(mode='draft', settings=settings, voice_parameters=native))[0]['result']
        assert planned['status'] == 'ready' and planned['evidence'] == 'planned' and planned['plan_id'] is None
        rows = {p['id']: p for p in planned['parameters']}
        assert rows['gender']['value'] is None and not rows['gender']['read_back']
        assert rows['head_size']['value'] == 70 and rows['speed']['value'] == 80
        assert rows['volume']['masked_native'] and rows['volume']['origin'] == 'context_mapping'
        assert synth(s, SETTINGS, None)[1] == baseline
        started, _ = synth(s, settings, native)
        application = started['native_application']
        assert application['status'] == 'applied' and application['identity'] == identity
        assert application['masked_parameters'] == ['volume']
        applied = s.request('explain_voice_parameters_v1', source=dict(mode='applied', plan_id=application['plan_id']))[0]['result']
        assert applied['evidence'] == 'adapter_applied' and all(row['read_back'] for row in applied['parameters'])
        actual = {p['id']: p for p in applied['parameters']}
        for key, row in rows.items():
            if row['value'] is not None:
                assert actual[key]['value'] == row['value'], (key, actual[key], row)
        assert synth(s, SETTINGS, None)[1] == baseline
        cases += 3
        # Each physical preset accepts edits and verified reset-to-default.
        first_plan = application['plan_id']
        for voice in range(1, 9):
            opts = dict(SETTINGS, voice_id=f'v{voice}')
            ordinary = synth(s, opts, None)[1]
            for operation in ({'op': 'set', 'value': 60}, {'op': 'default'}):
                started, _ = synth(s, opts, patch(identity, {'head_size': operation}), text='Test.')
                response = s.request('explain_voice_parameters_v1', source=dict(mode='applied', plan_id=started['native_application']['plan_id']))[0]['result']
                row = next(p for p in response['parameters'] if p['id'] == 'head_size')
                assert row['origin'] == ('native_set' if operation['op'] == 'set' else 'native_default') and row['read_back']
                if operation['op'] == 'set': assert row['value'] == 60
                cases += 1
            assert synth(s, opts, None)[1] == ordinary
            cases += 1
        # Malformed native data may never degrade to ordinary speech.
        bad = []
        for value in ('60', True, 60.5, -1, 101, None, {}, []):
            bad.append(patch(identity, {'head_size': dict(op='set', value=value)}, policy='common_only'))
        bad += [patch(identity, {'unknown': dict(op='default')}, policy='common_only'), patch(identity, context=['richness', 'richness']), patch(identity, context=['bogus'])]
        for field in ('native', 'context_dimensions', 'expected_identity', 'unavailable_policy'):
            value = patch(identity); del value[field]; bad.append(value)
        value = patch(identity); value['extra'] = True; bad.append(value)
        value = patch(identity); value['expected_identity']['runtime_generation'] = 1.0; bad.append(value)
        for data in bad:
            frames = s.request('synthesize', text='Invalid.', settings=SETTINGS, anchors=[], voice_parameters=data)
            assert len(frames) == 1 and frames[0]['type'] == 'error', frames
            assert frames[0]['code'] in ('invalid_parameter', 'invalid_request'), frames
            cases += 1
        # Identity changes are distinct from malformed patches.
        stale = patch(identity, {'head_size': dict(op='set', value=70)})
        stale['expected_identity']['runtime_generation'] += 2
        assert s.request('synthesize', text='Stale.', settings=SETTINGS, anchors=[], voice_parameters=stale)[0]['type'] == 'error'
        stale['unavailable_policy'] = 'common_only'
        started, ordinary = synth(s, SETTINGS, stale)
        assert started['native_application']['status'] == 'common_only' and ordinary == baseline
        assert s.request('explain_voice_parameters_v1', source=dict(mode='applied', plan_id='expired'))[0]['result']['reason'] == 'plan_expired'
        cases += 3
        # Common optional settings use the same omission semantics as Rust serde.
        minimal = {k: v for k, v in SETTINGS.items() if k not in ('voice_id', 'pitch_range', 'stress', 'richness')}
        synth(s, minimal, patch(identity))
        draft = s.request('explain_voice_parameters_v1', source=dict(mode='draft', settings=minimal, voice_parameters=None))[0]
        assert draft['result']['status'] == 'ready'
        cases += 1
        # Raw integer-token and duplicate-key failures recover on the same connection.
        for field, invalid in [('request_id', '1.0'), ('request_id', 'true'), ('protocol_version', '\"6\"')]:
            s.sequence += 1
            raw = json.dumps(dict(protocol_version=6, request_id=s.sequence, type='ping'))
            original = f'"{field}": ' + ('6' if field == 'protocol_version' else str(s.sequence))
            raw = raw.replace(original, f'"{field}": {invalid}')
            s.process.stdin.write(raw + '\n'); s.process.stdin.flush()
            assert s.frames.get(timeout=5)['type'] == 'error'
            assert s.request('ping')[0]['type'] == 'pong'
            cases += 1
        # Bounded receipt history: 65 later applications retire old plans.
        for i in range(65):
            last, _ = synth(s, SETTINGS, patch(identity), text='Test.')
        assert s.request('explain_voice_parameters_v1', source=dict(mode='applied', plan_id=first_plan))[0]['result']['reason'] == 'plan_expired'
        assert s.request('explain_voice_parameters_v1', source=dict(mode='applied', plan_id=last['native_application']['plan_id']))[0]['result']['status'] == 'ready'
        cases += 1
        streaming = streaming_cases(s, identity)
        result = dict(cases=cases + 2, helper_sha256=hashlib.sha256(program.read_bytes()).hexdigest(), catalogue=catalogue, exchanges=s.evidence, streaming=streaming)
    finally:
        s.close()
    # New runtime incarnation invalidates evidence but keeps metadata revision stable.
    s = Session(program, dll)
    try:
        new = s.catalogue()['identity']
        assert new['runtime_generation'] != identity['runtime_generation']
        assert new['catalogue_revision'] == identity['catalogue_revision']
        assert s.request('synthesize', text='Stale.', settings=SETTINGS, anchors=[], voice_parameters=patch(identity))[0]['type'] == 'error'
        result['cases'] += 1
    finally:
        s.close()
    s = Session(program, dll, version=5)
    try:
        start, _ = synth(s, SETTINGS, None)
        assert 'native_application' not in start
        rejected = s.request('get_engine_parameters_v1', engine_id='eloquence', voice_id=None, cursor=None, expected_catalogue_revision=None)[0]
        assert rejected['type'] == 'error'
        synth(s, SETTINGS, None)
        result['cases'] += 1
    finally:
        s.close()
    return result

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('program', type=pathlib.Path)
    parser.add_argument('--dll', default=r'C:\Program Files (x86)\Freedom Scientific\Shared\Eloquence\6.1\ECI.DLL')
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    result = run(args.program.resolve(), args.dll)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(f"PASS: {result['cases']} helper-6 Eloquence acceptance cases; evidence: {args.output}")
