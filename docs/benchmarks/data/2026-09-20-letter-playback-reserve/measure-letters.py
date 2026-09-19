#!/usr/bin/env python3
"""Measure actual legacy letter admission-to-mixer timing with muted device output.

This diagnostic requires the correlated mixer_source_started log introduced for
letter requests. It measures source consumption, not physical acoustic onset.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import queue
import re
import statistics
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('output', type=Path)
p.add_argument('--program', required=True)
p.add_argument('--engine', default='dectalk')
p.add_argument('--voice', default='paul')
p.add_argument('--rate', default='85')
p.add_argument('--scale', default='1.1')
p.add_argument('--iterations', type=int, default=20)
p.add_argument('--letters', default='abwA')
p.add_argument('--backend', choices=['device', 'null'], default='device')
p.add_argument('--gap-ms', type=float, default=300)
a = p.parse_args()
if a.iterations < 1 or a.gap_ms < 0:
    p.error('iterations must be positive and gap nonnegative')
a.output.mkdir(parents=True, exist_ok=False)
os.environ.update(
    OMNIVOX_PROGRAM=a.program,
    OMNIVOX_DECTALK_DLL='C:/Users/bart/AppData/Local/Omnivox/runtimes/dectalk/x86/DECtalk.dll',
    ESPEAK_NG_DATA='C:/Users/bart/AppData/Local/Emacsvox/Omnivox/espeak-data/9a2e176369cfe0b982737abf6c26037f21d2b9af2a2fabcc08062864d0157718',
    RUST_LOG='info,omnivox_audio::output=debug', EMACSVOX_OMNIVOX_DIAGNOSTIC='1',
    OMNIVOX_LOG_DIRECTORY=str(a.output / 'logs'),
)
spec = importlib.util.spec_from_file_location('benchmark', '/home/bart/src/omnivox/tools/benchmark_server.py')
b = importlib.util.module_from_spec(spec)
spec.loader.exec_module(b)


def integer(line, key):
    match = re.search(r'\b' + re.escape(key) + r'=(?:Some\()?([0-9]+)', line)
    return int(match.group(1)) if match else None


class Session(b.ServerSession):
    def __init__(self):
        self.events = queue.Queue()
        super().__init__(['/home/bart/src/emacsvox/servers/omnivox', '--audio-output', a.backend], a.engine, 20)

    def _read_stderr(self):
        for line in self.process.stderr:
            self.stderr_lines.append(line)
            if 'request_kind="letter"' in line:
                self.events.put(line)

    def measure(self, letter, epoch):
        sent = self.send_line('l {' + letter + '}')
        deadline = time.monotonic() + 20
        source = worker = None
        route = None
        while source is None or worker is None:
            line = self.events.get(timeout=max(.001, deadline - time.monotonic()))
            if integer(line, 'stop_epoch') != epoch:
                continue
            if 'lifecycle_stage="mixer_source_started"' in line:
                source = integer(line, 'admission_to_mixer_source_us') / 1000
            if 'lifecycle_stage="worker_finished"' in line:
                worker = {name.replace('_us', '_ms'): integer(line, name) / 1000 if integer(line, name) is not None else None
                          for name in ['worker_elapsed_us', 'admission_elapsed_us', 'admission_to_audio_queued_us']}
            if 'lifecycle_stage="synthesis_started"' in line:
                route = {name: re.search(r'\b' + name + r'="([^"]+)"', line).group(1)
                         for name in ['engine_id', 'voice_id']}
        if route != {'engine_id': a.engine, 'voice_id': a.voice}:
            raise RuntimeError(f'unexpected route: {route}')
        elapsed_ms = (time.perf_counter_ns() - sent) / 1_000_000
        time.sleep(max(0, (a.gap_ms - elapsed_ms) / 1000))
        return {'letter': letter, 'stop_epoch': epoch, 'source_ms': source, **worker, **route}


session = Session()
rows = []
try:
    cap, _ = session.negotiate(10000)
    b.configure_preferred_engine(session, cap, a.engine, 10001)
    for command in ['tts_set_voice ' + a.voice, 'tts_set_speech_rate ' + a.rate,
                    'tts_set_character_scale ' + a.scale, 'tts_set_voice_volume 0']:
        session.send_line(command)
    epoch = 0
    for letter in a.letters:
        for i in range(a.iterations + 3):
            epoch += 1
            row = session.measure(letter, epoch)
            if i >= 3:
                rows.append(row)
finally:
    session.close()
    (a.output / 'stderr.log').write_text(''.join(session.stderr_lines))

summary = {}
for letter in a.letters:
    values = sorted(row['source_ms'] for row in rows if row['letter'] == letter)
    summary[letter] = {'count': len(values), 'median_ms': statistics.median(values),
                       'p95_ms': b.nearest_rank(values, .95), 'maximum_ms': max(values)}
report = {'settings': {**vars(a), 'output': str(a.output)}, 'summary': summary, 'samples': rows}
(a.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(summary, indent=2))
