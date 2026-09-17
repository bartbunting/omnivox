#!/usr/bin/env python3
# Copyright (C) 2026 Bart Bunting
# SPDX-License-Identifier: GPL-2.0-or-later
"""Silent DECtalk helper-6 wire acceptance against a user-installed runtime."""
import argparse
import copy
import hashlib
import json
import pathlib
import time

from test_helper6_eloquence import Session, synth

SETTINGS = dict(voice_id='paul', rate=0.5, pitch=1.0, pitch_range=None,
                stress=None, richness=None, volume=1.0)
VOICES = ('paul', 'betty', 'harry', 'frank', 'kit', 'rita', 'ursula', 'dennis', 'wendy')


def catalogue(session, voice=None):
    deadline = time.monotonic() + 12
    while True:
        response = session.request('get_engine_parameters_v1', engine_id='dectalk',
            voice_id=voice, cursor=None, expected_catalogue_revision=None)[0]
        assert response['type'] == 'engine_parameters_v1', response
        result = response['result']
        if result['status'] != 'busy':
            assert result['status'] == 'ready', result
            return result
        assert time.monotonic() < deadline, result
        time.sleep(0.05)


def patch(identity, operations=None, context=None, policy='require'):
    return dict(native=dict(engine_id='dectalk', schema_id=identity['schema_id'], parameters=operations or {}),
        context_dimensions=context or [], expected_identity=copy.deepcopy(identity), unavailable_policy=policy)


def explain(session, settings, parameters):
    response = session.request('explain_voice_parameters_v1',
        source=dict(mode='draft', settings=settings, voice_parameters=parameters))[0]
    assert response['type'] == 'voice_parameters_explained_v1', response
    result = response['result']
    assert result['status'] == 'ready' and result['evidence'] == 'planned' and result['plan_id'] is None, result
    assert all(not row['read_back'] for row in result['parameters'])
    return {row['id']: row for row in result['parameters']}


def applied(session, started):
    receipt = started['native_application']
    assert receipt['status'] == 'applied', receipt
    response = session.request('explain_voice_parameters_v1', source=dict(mode='applied', plan_id=receipt['plan_id']))[0]
    result = response['result']
    assert result['status'] == 'ready' and result['evidence'] == 'adapter_applied', result
    assert result['plan_id'] == receipt['plan_id'] and result['identity'] == receipt['identity']
    assert len(result['parameters']) == 28 and all(row['read_back'] for row in result['parameters'])
    return {row['id']: row for row in result['parameters']}


def compare(planned, actual):
    for key, row in planned.items():
        assert row['origin'] == actual[key]['origin'] and row['masked_native'] == actual[key]['masked_native'], (key, row, actual[key])
        if row['value'] is not None:
            assert row['value'] == actual[key]['value'], (key, row, actual[key])


def streaming(session, identity):
    frames = session.request('synthesize', text='Native marker test.', settings=SETTINGS,
        anchors=[dict(id='start', text_offset=0, affinity='before'), dict(id='end', text_offset=19, affinity='after')],
        voice_parameters=patch(identity, {'hs': dict(op='set', value=110)}))
    assert frames[0]['type'] == 'synthesis_started' and frames[-1]['type'] == 'synthesis_completed', frames[-1]
    markers = [m for f in frames if f['type'] == 'markers' for m in f['markers']]
    anchors = [m for m in markers if m['kind'] == 'requested_anchor']
    assert {m['value'] for m in anchors} == {'start', 'end'}, anchors
    request = session.send('synthesize', text='Cancellation sample. ' * 500, settings=SETTINGS,
        anchors=[], voice_parameters=patch(identity, {'hs': dict(op='set', value=110)}))
    first = session.frames.get(timeout=15)
    assert first['request_id'] == request['request_id'] and first['type'] == 'synthesis_started', first
    query = session.send('get_engine_parameters_v1', engine_id='dectalk', voice_id='paul', cursor=None, expected_catalogue_revision=None)
    cancel = session.send('cancel', target_request_id=request['request_id'])
    accepted = False
    queried = False
    kinds = []
    while True:
        frame = session.frames.get(timeout=15)
        kind = frame['type']
        if frame['request_id'] == query['request_id']:
            assert kind == 'engine_parameters_v1' and frame['result']['status'] == 'ready', frame
            queried = True
        elif frame['request_id'] == cancel['request_id']:
            assert kind == 'cancel_accepted', frame
            accepted = True
        else:
            assert frame['request_id'] == request['request_id'], frame
            if accepted:
                assert kind == 'synthesis_cancelled', frame
            assert kind not in ('error', 'synthesis_completed'), frame
            if kind == 'synthesis_cancelled':
                assert accepted and queried
                kinds.append(kind)
                break
        if kind != 'audio_chunk':
            kinds.append(kind)
    ordinary = synth(session, SETTINGS, None)
    started, _ = synth(session, SETTINGS, patch(identity))
    actual = applied(session, started)
    return dict(anchors=anchors, cancellation_frames=kinds, ordinary_followup_frames=ordinary[1],
        followup_readback_count=len(actual))


def run(program, dll):
    session = Session(program, dll)
    cases = 0
    try:
        cat = catalogue(session)
        identity = cat['identity']
        assert len(cat['parameters']) == 28 and cat['next_cursor'] is None
        assert all(row['default'] == dict(source='unknown', value=None, reset_supported=True) for row in cat['parameters'])
        assert catalogue(session, 'paul')['identity'] == identity
        assert {row['id'] for row in cat['parameters']} == {
            'sx', 'sm', 'as', 'ap', 'pr', 'br', 'ri', 'nf', 'la', 'hs', 'f4', 'b4', 'f5', 'b5',
            'gf', 'gh', 'gv', 'gn', 'g1', 'g2', 'g3', 'g4', 'g5', 'bf', 'lx', 'qu', 'hr', 'sr'}
        cases += 1
        # Each scalar boundary passes through the wire, native application and readback.
        for descriptor in cat['parameters']:
            for bound in ('minimum', 'maximum'):
                key = descriptor['id']
                value = descriptor['value_type'][bound]
                native = patch(identity, {key: dict(op='set', value=value)})
                plan = explain(session, SETTINGS, native)
                started, _ = synth(session, SETTINGS, native, text='Test.')
                actual = applied(session, started)
                compare(plan, actual)
                assert actual[key]['value'] == value and actual[key]['origin'] == 'native_set'
                cases += 1
        # Every voice: preserved common clamps, native overrides, complete defaults and context.
        first_plan = started['native_application']['plan_id']
        for voice in VOICES:
            settings = dict(SETTINGS, voice_id=voice)
            baseline = synth(session, settings, None)[1]
            for level, pitch in ((0.0, 2.0), (0.5, 1.0), (1.0, 0.5)):
                common = dict(settings, pitch=pitch, pitch_range=level, stress=level, richness=level)
                native = patch(identity, {'sm': dict(op='set', value=55), 'ap': dict(op='set', value=75), 'hr': dict(op='set', value=50)},
                    ['richness', 'average_pitch', 'stress'])
                plan = explain(session, common, native)
                assert plan['hs']['value'] is None and not plan['hs']['read_back']
                started, _ = synth(session, common, native, text='Test.')
                compare(plan, applied(session, started))
                assert started['native_application']['masked_parameters'] == ['sm', 'ap', 'hr']
                if level == 0:
                    assert plan['hr']['value'] == 2 and plan['sr']['value'] == 1
                    if voice == 'kit':
                        assert plan['ap']['value'] == 350
                cases += 1
            # All unknown numerical defaults remain selectable and verified at application.
            native = patch(identity, {row['id']: dict(op='default') for row in cat['parameters']})
            plan = explain(session, settings, native)
            assert all(row['value'] is None and row['origin'] == 'native_default' for row in plan.values())
            started, _ = synth(session, settings, native, text='Test.')
            actual = applied(session, started)
            assert all(row['origin'] == 'native_default' for row in actual.values())
            assert synth(session, settings, None)[1] == baseline
            cases += 1
        # Rate and PCM volume do not write these 28 design-voice controls.
        native = patch(identity, {'g5': dict(op='set', value=60), 'ap': dict(op='set', value=150)}, ['volume', 'rate', 'rate_offset'])
        started, _ = synth(session, SETTINGS, native)
        actual = applied(session, started)
        assert started['native_application']['masked_parameters'] == []
        assert actual['g5']['value'] == 60 and actual['ap']['value'] == 150
        cases += 1
        # Invalid controls are rejected even with context masking and common-only permission.
        bad = []
        for value in ('55', True, 55.5, -1, 101, None, {}, []):
            bad.append(patch(identity, {'sm': dict(op='set', value=value)}, ['richness'], 'common_only'))
        for operations in ({'unknown': dict(op='default')}, {'ap': dict(op='set', value=500)}, {'hr': dict(op='set', value=0)}, {'sr': dict(op='set', value=0)}):
            bad.append(patch(identity, operations, policy='common_only'))
        bad += [patch(identity, context=['richness', 'richness']), patch(identity, context=['unknown'])]
        for field in ('native', 'context_dimensions', 'expected_identity', 'unavailable_policy'):
            data = patch(identity)
            del data[field]
            bad.append(data)
        data = patch(identity)
        data['extra'] = True
        bad.append(data)
        for data in bad:
            frames = session.request('synthesize', text='Invalid.', settings=SETTINGS, anchors=[], voice_parameters=data)
            assert len(frames) == 1 and frames[0]['type'] == 'error', frames
            cases += 1
        stale = patch(identity, {'sm': dict(op='set', value=55)})
        stale['expected_identity']['runtime_generation'] += 2
        rejected = session.request('synthesize', text='Stale.', settings=SETTINGS, anchors=[], voice_parameters=stale)
        assert len(rejected) == 1 and rejected[0]['type'] == 'error'
        stale['unavailable_policy'] = 'common_only'
        started, _ = synth(session, SETTINGS, stale)
        assert started['native_application']['status'] == 'common_only'
        minimal = {key: value for key, value in SETTINGS.items() if key not in ('voice_id', 'pitch_range', 'stress', 'richness')}
        synth(session, minimal, patch(identity))
        cases += 3
        for _ in range(65):
            last, _ = synth(session, SETTINGS, patch(identity), text='Test.')
        expired = session.request('explain_voice_parameters_v1', source=dict(mode='applied', plan_id=first_plan))[0]['result']
        assert expired['reason'] == 'plan_expired'
        applied(session, last)
        cases += 1
        stream = streaming(session, identity)
        cases += 2
        result = dict(cases=cases, helper_sha256=hashlib.sha256(program.read_bytes()).hexdigest(), catalogue=cat,
            exchanges=session.evidence, streaming=stream)
    finally:
        session.close()
    session = Session(program, dll)
    try:
        current = catalogue(session)['identity']
        assert current['runtime_generation'] != identity['runtime_generation']
        assert current['catalogue_revision'] == identity['catalogue_revision']
        assert session.request('synthesize', text='Stale.', settings=SETTINGS, anchors=[], voice_parameters=patch(identity))[0]['type'] == 'error'
        result['cases'] += 1
    finally:
        session.close()
    session = Session(program, dll, version=5)
    try:
        started, _ = synth(session, SETTINGS, None)
        assert 'native_application' not in started
        assert session.request('get_engine_parameters_v1', engine_id='dectalk', voice_id=None, cursor=None,
            expected_catalogue_revision=None)[0]['type'] == 'error'
        synth(session, SETTINGS, None)
        result['cases'] += 1
    finally:
        session.close()
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('program', type=pathlib.Path)
    parser.add_argument('--dll', default=r'C:\Users\bart\AppData\Local\Omnivox\runtimes\dectalk\x86\DECtalk.dll')
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    result = run(args.program.resolve(), args.dll)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(f"PASS: {result['cases']} DECtalk helper-6 acceptance cases; evidence: {args.output}")
