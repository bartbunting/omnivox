"""Summarize retained DECtalk diagnostics; never compare wall clocks across processes."""
import gzip
import json
import math
from pathlib import Path
import re
import statistics
import sys

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).parent


def stats(values):
    values = sorted(values)
    return dict(n=len(values), median_ms=statistics.median(values),
                p95_ms=values[math.ceil(len(values) * .95) - 1],
                minimum_ms=values[0], maximum_ms=values[-1])


def log_text(path):
    return gzip.decompress(path.read_bytes()).decode() if path.suffix == '.gz' else path.read_text()


def comparison(directory):
    groups = {}
    for job in json.loads((directory / 'manifest.json').read_text())['jobs']:
        raw = json.loads((directory / job['report']).read_text())
        groups.setdefault(job['target'], []).extend(
            s for s in raw['timing_samples'] if s['case'] == 'line')
    return {target: {field: stats([s[field] for s in samples])
                    for field in ('dispatch_to_source_ms', 'dispatch_to_terminal_ms')}
            for target, samples in groups.items()}


observations = []
for directory in sorted((root / 'repeated').glob('*-probe512-*')):
    raw = json.loads((directory / 'raw.json').read_text())
    samples = {s['dispatch_id']: s for s in raw['timing_samples'] if s['case'] == 'line'}
    helper_to_dispatch, probes, terminals = {}, {}, {}
    for path in sorted((directory / 'server-logs').glob('*.log*')):
        for line in log_text(path).splitlines():
            if 'Starting routed synthesis' in line:
                dispatch = int(re.search(r'request_identifier=Some\((\d+)\)', line)[1])
            if 'Sending progressive TTS helper synthesis request' in line:
                request = int(re.search(r'request_id=(\d+)', line)[1])
                assert request not in helper_to_dispatch
                helper_to_dispatch[request] = dispatch
            if 'helper_event=dectalk_latency_probe ' in line:
                probe = {k: int(v) for k, v in re.findall(r'(\w+)=(-?\d+)\b', line)}
                assert probe['request_id'] not in probes
                probes[probe['request_id']] = probe
            if 'lifecycle_stage="playback_terminal"' in line:
                identifier = int(re.search(r'request_identifier=(\d+)', line)[1])
                queued = int(re.search(r'admission_to_audio_queued_us=Some\((\d+)\)', line)[1])
                mixer = int(re.search(r'admission_to_mixer_source_us=Some\((\d+)\)', line)[1])
                terminals[identifier] = mixer - queued
    matched = []
    for request, dispatch in helper_to_dispatch.items():
        if dispatch not in samples:
            continue
        observations.append(dict(job=str(directory.relative_to(root)),
                                 dispatch_id=dispatch, request_id=request,
                                 probe=probes[request], client=samples[dispatch],
                                 queued_to_mixer_us=terminals[dispatch]))
        matched.append(dispatch)
    assert set(matched) == set(samples) and len(matched) == len(samples)

phases = {
    'native_reset': ('reset_begin_us', 'reset_end_us'),
    'set_rate': ('rate_begin_us', 'rate_end_us'),
    'build_indexed_text': ('text_begin_us', 'text_ready_us'),
    'speak_call': ('speak_begin_us', 'speak_end_us'),
    'speak_to_first_callback': ('speak_begin_us', 'cb0_us'),
    'first_callback_to_first_audio_write': ('cb0_us', 'sink_begin_us'),
    'first_audio_write': ('sink_begin_us', 'sink_end_us'),
    'native_sync_overlaps_audio_delivery': ('sync_begin_us', 'sync_end_us'),
}
summary = dict(
    units='milliseconds; medians and nearest-rank p95; phases can overlap',
    phases={name: stats([(o['probe'][end] - o['probe'][start]) / 1000
                        for o in observations]) for name, (start, end) in phases.items()},
    queued_to_mixer=stats([o['queued_to_mixer_us'] / 1000 for o in observations]),
    first_callback_minimum_peak=min(o['probe']['cb0_peak'] for o in observations),
    first_callback_frames=sorted(set(o['probe']['cb0_frames'] for o in observations)),
    instrumentation_control=comparison(root / 'repeated'),
    reset_after_comparison=comparison(root / 'reset-after-comparison'))
if (root / 'native-comparison/manifest.json').exists():
    summary['native_comparison'] = comparison(root / 'native-comparison')
    native_probes = []
    for directory in sorted((root / 'native-comparison').glob('*-reset-after-*')):
        lines = [line for path in sorted((directory / 'server-logs').glob('*.log*'))
                 for line in log_text(path).splitlines()
                 if 'helper_event=dectalk_latency_probe ' in line]
        assert len(lines) == 33  # Three warmups, then thirty measured requests.
        native_probes.extend({k: int(v) for k, v in re.findall(r'(\w+)=(-?\d+)\b', line)}
                             for line in lines[3:])
    summary['native_parameter_preparation_after_reset_move'] = stats([
        (p['text_begin_us'] - p['rate_end_us']) / 1000 for p in native_probes])
(root / 'observations.json').write_text(json.dumps(observations, indent=2) + '\n')
(root / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary, indent=2))
