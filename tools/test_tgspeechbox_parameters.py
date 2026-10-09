#!/usr/bin/env python3
"""Exercise native TGSpeechBox controls through the actual staged helper."""
import argparse
import array
import base64
import json
import math
import os
import queue
import subprocess
import threading
import time
from pathlib import Path

class Helper:

    def __init__(self, program, version=6, rate=44100):
        env = dict(os.environ, OMNIVOX_TGSPEECHBOX_SAMPLE_RATE=str(rate))
        if str(program).endswith('.exe') and os.name != 'nt':
            forwarded = [v for v in env.get('WSLENV', '').split(':') if v]
            forwarded.append('OMNIVOX_TGSPEECHBOX_SAMPLE_RATE')
            env['WSLENV'] = ':'.join(forwarded)
        self.p = subprocess.Popen([str(program)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
        self.q = queue.Queue()
        self.errors = []
        self.id = 0
        self.version = version

        def output():
            for line in self.p.stdout:
                self.q.put(json.loads(line))
            self.q.put(None)

        def errors():
            for line in self.p.stderr:
                self.errors.append(line.decode(errors='replace'))
        self.reader = threading.Thread(target=output, daemon=True)
        self.reader.start()
        self.errreader = threading.Thread(target=errors, daemon=True)
        self.errreader.start()
        self.send('hello', supported_protocol_versions=[version])
        assert self.read()['selected_protocol_version'] == version

    def send(self, kind, **kw):
        self.id += 1
        self.p.stdin.write((json.dumps(dict(protocol_version=self.version, request_id=self.id, type=kind, **kw)) + '\n').encode())
        self.p.stdin.flush()
        return self.id

    def read(self):
        item = self.q.get(timeout=30)
        assert item is not None, ''.join(self.errors)
        return item

    def call(self, kind, **kw):
        rid = self.send(kind, **kw)
        reply = self.read()
        assert reply['request_id'] == rid, reply
        return reply

    def catalogue(self, voice='en-us/adam'):
        reply = self.call('get_engine_parameters_v1', engine_id='tgspeechbox', voice_id=voice, cursor=None, expected_catalogue_revision=None)
        assert reply['result']['status'] == 'ready', reply
        return reply['result']

    def synth(self, p=None, voice='en-us/adam', text='We speak clearly about a warm summer morning.', fail=False):
        args = dict(text=text, settings=dict(voice_id=voice, rate=0.5, pitch=1.0, volume=0.8, pitch_range=0.5, stress=None, richness=None), anchors=[dict(id='start', text_offset=0, affinity='before'), dict(id='end', text_offset=len(text.encode()), affinity='after')])
        if self.version == 6:
            args['voice_parameters'] = p
        rid = self.send('synthesize', **args)
        pcm = bytearray()
        start = None
        marks = []
        seq = 0
        while True:
            reply = self.read()
            assert reply['request_id'] == rid, reply
            kind = reply['type']
            if kind == 'error':
                assert fail and (not pcm) and (start is None), reply
                return reply
            if kind == 'synthesis_started':
                assert start is None
                start = reply
            elif kind == 'audio_chunk':
                assert start is not None
                assert reply['chunk']['sequence'] == seq
                seq += 1
                pcm.extend(base64.b64decode(reply['chunk']['data_base64']))
            elif kind == 'markers':
                marks.extend(reply['markers'])
            elif kind == 'anchors':
                marks.extend(reply['anchors'])
            elif kind == 'synthesis_completed':
                assert not fail and pcm and start
                assert len(pcm) == reply['frame_count'] * start['format']['channels'] * 2
                anchors = {m['value']: m['frame_offset'] for m in marks if m['kind'] == 'requested_anchor'}
                assert anchors.get('start') == 0 and anchors.get('end') == reply['frame_count'], marks
                return (start, bytes(pcm), marks)

    def close(self):
        if self.p.poll() is None:
            try:
                self.call('shutdown')
                self.p.wait(timeout=5)
            finally:
                if self.p.poll() is None:
                    self.p.kill()
                    self.p.wait()
        self.reader.join(timeout=2)
        self.errreader.join(timeout=2)

    def cancel_native(self, parameters):
        rid = self.send(
            'synthesize',
            text='A long sentence to exercise cancellation. ' * 100,
            settings=dict(voice_id='en-us/adam', rate=0.5, pitch=1.0, volume=0.8),
            anchors=[],
            voice_parameters=parameters,
        )
        cancel = self.send('cancel', target_request_id=rid)
        acknowledged = False
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            reply = self.read()
            if reply['request_id'] == cancel:
                assert reply['type'] == 'cancel_accepted', reply
                acknowledged = True
            elif reply['request_id'] == rid:
                if reply['type'] == 'synthesis_cancelled':
                    assert acknowledged
                    return
                assert not acknowledged, reply
                assert reply['type'] in (
                    'synthesis_started', 'audio_chunk', 'markers', 'anchors'
                ), reply
            else:
                raise AssertionError(reply)
        raise AssertionError('Native cancellation timed out')

def rms(pcm):
    a = array.array('h', pcm)
    return math.sqrt(sum((x * x for x in a)) / len(a))

def fresh_capture(program, rate, parameters=None):
    helper = Helper(program, rate=rate)
    try:
        return helper.synth(parameters)[1]
    finally:
        helper.close()

def probe(program, rate):
    h = Helper(program, rate=rate)
    try:
        c = h.catalogue()
        assert f'.dsp9.{rate}.' in c['identity']['profile_id'], c
        ids = [p['id'] for p in c['parameters']]
        assert set(ids) == {'breathiness', 'creakiness', 'brightness', 'jitter', 'shimmer'}, ids
        assert all((p['default']['value'] == 0 and p['default']['reset_supported'] for p in c['parameters']))
        p = dict(native=dict(engine_id='tgspeechbox', schema_id=c['identity']['schema_id'], parameters={}), expected_identity=c['identity'], context_dimensions=[], unavailable_policy='require')
        baseline = h.synth()[1]
        # Native noise generators advance between utterances. Compare the
        # first utterance of fresh processes so noise history cannot make an
        # ignored control look effective merely by changing the PCM bytes.
        assert fresh_capture(program, rate) == baseline
        beth_baseline = h.synth(voice='en-us/beth')[1]
        results = {}
        for key, value in [('breathiness', 0.4), ('creakiness', 0.4), ('brightness', 8.0), ('jitter', 0.3), ('shimmer', 0.4)]:
            p['native']['parameters'] = {key: dict(op='set', value=value)}
            start, pcm, marks = h.synth(p)
            app = start['native_application']
            assert app['status'] == 'applied', app
            e = h.call('explain_voice_parameters_v1', source=dict(mode='applied', plan_id=app['plan_id']))['result']
            actual = {v['id']: v['value'] for v in e['parameters']}
            assert e['evidence'] == 'adapter_applied' and actual[key] == value, e
            cold_pcm = fresh_capture(program, rate, p)
            assert cold_pcm != baseline, (key, 'control produced unchanged PCM')
            p['native']['parameters'] = {key: dict(op='default')}
            start, neutral, _ = h.synth(p)
            e = h.call('explain_voice_parameters_v1', source=dict(mode='applied', plan_id=start['native_application']['plan_id']))['result']
            assert all((v['value'] == 0 for v in e['parameters'])), e
            ratio = rms(neutral) / rms(baseline)
            assert 0.85 < ratio < 1.15, (key, 'neutral loudness drift', ratio)
            results[key] = dict(
                changed_rms=rms(pcm), fresh_control_rms=rms(cold_pcm),
                neutral_rms=rms(neutral), neutral_ratio=ratio,
                frames=len(pcm) // 4, matched_fresh_process_pcm='different',
            )
        p['native']['parameters'] = {
            key: dict(op='set', value=value)
            for key, value in [('breathiness', 0.5), ('brightness', 8.0), ('jitter', 0.4)]
        }
        h.synth(p)
        ordinary = h.synth(voice='en-us/beth')[1]
        assert 0.85 < rms(ordinary) / rms(beth_baseline) < 1.15
        h.cancel_native(p)
        ordinary = h.synth(voice='en-us/beth')[1]
        cancellation_reset_ratio = rms(ordinary) / rms(beth_baseline)
        assert 0.85 < cancellation_reset_ratio < 1.15
        for key, value in [('breathiness', 1.1), ('brightness', -12.1), ('unknown', 0.2)]:
            p['native']['parameters'] = {key: dict(op='set', value=value)}
            h.synth(p, fail=True)
        p['native']['parameters'] = {}
        p['expected_identity'] = dict(c['identity'], runtime_generation=c['identity']['runtime_generation'] + 1)
        h.synth(p, fail=True)
        p['unavailable_policy'] = 'common_only'
        start, _, _ = h.synth(p)
        assert start['native_application']['status'] == 'common_only'
        h.synth()
        return dict(
            rate=rate, identity=c['identity'], baseline_rms=rms(baseline),
            controls=results, cancellation_reset_ratio=cancellation_reset_ratio,
            voice_switch_reset='passed',
        )
    finally:
        h.close()

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('helper', type=Path)
    ap.add_argument('--output', type=Path)
    args = ap.parse_args()
    result = [probe(args.helper.resolve(), rate) for rate in (44100, 22050)]
    h = Helper(args.helper.resolve(), version=5)
    try:
        start, pcm, _ = h.synth()
        assert 'native_application' not in start and pcm
    finally:
        h.close()
    payload = json.dumps(dict(results=result, legacy_v5='passed'), indent=2)
    if args.output:
        args.output.write_text(payload + '\n')
    print(payload)
if __name__ == '__main__':
    main()
