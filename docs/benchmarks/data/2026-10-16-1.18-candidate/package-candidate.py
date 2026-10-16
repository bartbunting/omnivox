from pathlib import Path
import hashlib, tarfile, shutil
root=Path('/tmp/ov118/release')
out=Path('/home/bart/src/omnivox-release-1.18.0/target/release-candidate')
out.mkdir(parents=True, exist_ok=True)
shutil.copy2('elisp/omnivox-voices.el', root/'omnivox-voices.el')
archive=out/'omnivox-1.18.0-linux-x64.tar.gz'
with tarfile.open(archive,'w:gz',dereference=True) as bundle:
    for name in ('omnivox','LICENSE','LICENSING.md','omnivox-voices.el','espeak-ng-data','third-party-licenses','rhvoice','tgspeechbox'):
        bundle.add(root/name,arcname=name)
digest=hashlib.sha256(archive.read_bytes()).hexdigest()
(out/'sha256sums.txt').write_text(f'{digest}  {archive.name}\n')
print(archive)
print(digest)
