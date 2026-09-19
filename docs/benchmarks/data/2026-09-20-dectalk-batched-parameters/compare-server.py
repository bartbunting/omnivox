# Repeat the ordinary/custom comparison with the maintained server harness.
import argparse
import copy
import importlib.util
import json
import random
import sys
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('output', type=Path)
parser.add_argument('--emacsvox', type=Path, required=True)
parser.add_argument('--plan', type=Path, required=True)
parser.add_argument('--target', action='append', required=True, help='name=program')
parser.add_argument('--iterations', type=int, default=30)
parser.add_argument('--repeats', type=int, default=3)
a = parser.parse_args()
root = a.emacsvox.resolve()
spec = importlib.util.spec_from_file_location('benchmark', root / 'utils/voice_benchmark.py')
b = importlib.util.module_from_spec(spec)
spec.loader.exec_module(b)
plan = json.loads(a.plan.read_text())
out = a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
manifest = {'kind': 'warm ordinary/custom DECtalk; null audio; software onset',
            'seed': 71423, 'plan_sha256': b.digest(a.plan), 'targets': {}, 'jobs': [],
            'harness': {str(p): b.digest(p) for p in [
                root / 'utils/voice_benchmark.py', root / 'utils/voice_benchmark_server.py',
                Path(plan['omnivox_root']) / 'tools/benchmark_server.py', Path(__file__)]}}
route = next(r for r in plan['routes'] if r['engine'] == 'dectalk')
targets = []
for arg in a.target:
    name, program = arg.split('=', 1)
    t = copy.deepcopy(plan['targets'][1])
    t.update(id=name, program=program, native=False, ui=False)
    t['environment']['OMNIVOX_PROGRAM'] = program
    targets.append(t)
    directory = Path(program).parent
    manifest['targets'][name] = {
        'program': program, 'sha256': b.digest(program),
        'payload': {p.name: b.digest(p) for p in directory.iterdir()
                    if p.is_file() and p.suffix.lower() in ('.exe', '.dll')},
        'provenance': (directory / 'PROVENANCE').read_text()}
for repeat in range(a.repeats):
    jobs = [(t, flavour) for t in targets for flavour in ('legacy', 'native')]
    random.Random(manifest['seed'] + repeat).shuffle(jobs)
    for t, flavour in jobs:
        name = f"{len(manifest['jobs']):02}-{t['id']}-{flavour}"
        d = out / name
        d.mkdir()
        job = {'audio_output': 'null', 'cases': ['line'], 'concurrent': False,
               'flavour': flavour, 'iterations': a.iterations, 'mode': 'warm',
               'omnivox_root': plan['omnivox_root'], 'rate': 225, 'replacement_burst': 5,
               'route': route, 'server': t['server'], 'timeout': 30, 'warmups': 3}
        b.write_json(d / 'job.json', job)
        print(name, flush=True)
        b.run_child([sys.executable, str(root / 'utils/voice_benchmark_server.py'),
                     str(d / 'job.json'), str(d / 'raw.json')],
                    b.child_environment(t, d), d / 'worker.log', max(120, a.iterations * 30))
        raw = json.loads((d / 'raw.json').read_text())
        values = [s['dispatch_to_source_ms'] for s in raw['timing_samples'] if s['case'] == 'line']
        entry = {'target': t['id'], 'flavour': flavour, 'repeat': repeat,
                 'report': str((d / 'raw.json').relative_to(out)),
                 'sha256': b.digest(d / 'raw.json'), 'n': len(values),
                 'median': b.percentile(values, .5), 'p95': b.percentile(values, .95)}
        manifest['jobs'].append(entry)
        b.write_json(out / 'manifest.json', manifest)
        print(entry, flush=True)
summary = {}
for entry in manifest['jobs']:
    raw = json.loads((out / entry['report']).read_text())
    key = entry['target'] + '/' + entry['flavour']
    cell = summary.setdefault(key, {})
    for row in raw['timing_samples']:
        if row['case'] != 'line':
            continue
        for field in ('dispatch_to_source_ms', 'dispatch_to_terminal_ms'):
            if field in row:
                cell.setdefault(field, []).append(row[field])
summary = {key: {field: {'n': len(v), 'median': b.percentile(v, .5), 'p95': b.percentile(v, .95)}
                 for field, v in fields.items()} for key, fields in summary.items()}
b.write_json(out / 'summary.json', summary)
print(json.dumps(summary, indent=2))
