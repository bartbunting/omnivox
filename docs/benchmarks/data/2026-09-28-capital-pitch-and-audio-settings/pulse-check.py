from pathlib import Path
import json
import os
import re
import subprocess
import sys
import tempfile

program = Path(sys.argv[1]).resolve()
output = Path(sys.argv[2])
output.mkdir(parents=True, exist_ok=False)
with tempfile.TemporaryDirectory(prefix='omnivox-pulse-settings-') as temporary:
    root = Path(temporary)
    config = {'schema': 2, 'audio': {'backend': 'pulse', 'target': 'left', 'pulse_latency_ms': 45},
              'engine_overrides': {name: {'enabled': False} for name in
                 ['winrt', 'macos', 'piper', 'rhvoice', 'flite', 'rutts', 'tgspeechbox', 'eloquence', 'dectalk', 'mbrola']}}
    (root / 'config.json').write_text(json.dumps(config))
    (output / 'config.json').write_text(json.dumps(config, indent=2) + '\n')
    environment = dict(os.environ)
    for key in list(environment):
        if key.startswith('OMNIVOX_') or key == 'PULSE_LATENCY_MSEC':
            environment.pop(key)
    results = []
    for name, override, expected in [('saved', None, 45), ('environment', '30', 30)]:
        selected = dict(environment)
        if override is not None:
            selected['OMNIVOX_PULSE_LATENCY_MS'] = override
        command = [str(program), '--engine', 'espeak', '--config-dir', str(root)]
        # Empty stdin: construct output and drain/shut down without producing sound.
        process = subprocess.run(command, input='', capture_output=True, text=True, env=selected, timeout=40)
        (output / (name + '.stdout')).write_text(process.stdout)
        (output / (name + '.stderr')).write_text(process.stderr)
        requests = re.findall(r'requested_ms=(\d+)', process.stderr)
        passed = process.returncode == 0 and requests == [str(expected)] * 3
        results.append({'case': name, 'expected_ms': expected, 'observed_requests_ms': requests, 'exit_code': process.returncode, 'passed': passed})
    (output / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(results, indent=2))
    sys.exit(0 if all(row['passed'] for row in results) else 1)
