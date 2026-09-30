from pathlib import Path
import hashlib, json, subprocess, sys
sys.path.insert(0, '/home/bart/src/omnivox-issue-3-main/tools')
from verify_release_asset_set import require_exact_names, checksum_names
root = Path(__file__).parent
raw = subprocess.check_output(['gh','api','repos/bartbunting/omnivox/releases/tags/v1.16.0'])
root.joinpath('release.json').write_bytes(raw)
r = json.loads(raw)
assert not r['draft'] and not r['prerelease'] and r['published_at']
require_exact_names([a['name'] for a in r['assets']], '1.16.0')
subprocess.run(['gh','release','download','v1.16.0','--repo','bartbunting/omnivox','--pattern','sha256sums.txt','--dir',str(root),'--clobber'], check=True)
manifest = root/'sha256sums.txt'
assert len(checksum_names(manifest)) == 27
checks = {line.split()[1].lstrip('*'):line.split()[0] for line in manifest.read_text().splitlines()}
assert set(checks) == {a['name'] for a in r['assets']} - {'sha256sums.txt'}
for a in r['assets']:
    expected = hashlib.sha256(manifest.read_bytes()).hexdigest() if a['name']=='sha256sums.txt' else checks[a['name']]
    assert a['digest']=='sha256:'+expected, a['name']
refs = subprocess.check_output(['git','ls-remote','origin','refs/tags/v1.16.0','refs/tags/v1.16.0^{}'],text=True)
assert 'fbed2c9368746a4e35a1e356fcf8bfbb627c66cd\trefs/tags/v1.16.0^{}' in refs
root.joinpath('remote-tag.txt').write_text(refs)
print('PASS: public stable v1.16.0; exact 28 assets; all 27 package digests match manifest; manifest digest and remote tag verified')
print('Published:', r['published_at'])
