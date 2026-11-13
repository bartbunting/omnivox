#!/usr/bin/env python3
"""Retain synthetic private-profile observations, excluding owner credentials."""
from pathlib import Path
import gzip, hashlib, json, re, shutil

root = Path(__file__).resolve().parent
cases = []
for launch in sorted(Path('/tmp').glob('omnivox-discovery-*.log')):
    text = launch.read_text(errors='replace')
    match = re.search(r'Private profile: (.+)', text)
    if not match:
        continue
    profile = Path(match.group(1))
    name = launch.stem.removeprefix('omnivox-discovery-')
    destination = root / name
    destination.mkdir(exist_ok=True)
    shutil.copy2(launch, destination / 'launcher.log')
    for filename in ['runtime.json', 'config.json', 'saved-choices.el', 'gui-check.json',
                     'gui-check-error.txt', 'late-check.txt', 'late-error.txt', 'refresh-profile.el']:
        source = profile / filename
        if source.exists():
            shutil.copy2(source, destination / filename)
    for source in list((profile / 'logs').glob('*.log')) + [profile / 'aural.jsonl', profile / 'client-messages.txt']:
        if source.exists():
            (destination / (source.name + '.gz')).write_bytes(gzip.compress(source.read_bytes(), mtime=0))
    cases.append(dict(case=name, profile=str(profile),
                      passed=any((profile / f).exists() for f in ['gui-check.json', 'late-check.txt'])
                      or 'PASS reload' in text,
                      failed=any((profile / f).exists() for f in ['gui-check-error.txt', 'late-error.txt'])))
checks = root / 'checks'; checks.mkdir(exist_ok=True)
for name in ['source', 'focused', 'unit', 'bytecode', 'docs', 'headers']:
    source = Path('/tmp/omnivox-discovery-' + name + '.log')
    if source.exists():
        (checks / (name + '.log.gz')).write_bytes(gzip.compress(source.read_bytes(), mtime=0))
(root / 'runs.json').write_text(json.dumps(cases, indent=2) + '\n')
paths = sorted(p for p in root.rglob('*') if p.is_file() and p.name != 'SHA256SUMS' and '__pycache__' not in p.parts)
(root / 'SHA256SUMS').write_text(''.join(hashlib.sha256(p.read_bytes()).hexdigest() + '  ' + str(p.relative_to(root)) + '\n' for p in paths))
print(json.dumps(cases, indent=2))
