#!/usr/bin/env python3
import copy,json,os,re,sys
from pathlib import Path
sys.dont_write_bytecode=True
import probe
root=Path(sys.argv[2]).resolve();root.mkdir(parents=True);exe=Path(sys.argv[1]).resolve()
env={k:os.environ[k] for k in ['PATH','HOME','LANG'] if k in os.environ};env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='espeak',OMNIVOX_AUDIO_OUTPUT='null',OMNIVOX_EXPERIMENTAL_RELOAD_TOKEN=os.urandom(32).hex())
probe.save(root/'config.json',probe.configuration())
o=probe.Owner(exe,env)
try:
 o.send(b'tts_set_punctuation_profile probe\n');before=o.speech('hello!')
 candidate,_=probe.prepare(exe,env);candidate['punctuation']['profiles']['probe']['table']['!']='new pronunciation marker'
 applied=o.request(candidate);assert applied['outcome']=='applied';after=o.speech('hello!')
finally:o.close()
(root/'stderr.log').write_bytes(o.err)
counts={}
for line in o.err.decode().splitlines():
 if 'Starting routed synthesis' in line:
  request=re.search(r'request_identifier=Some\((\d+)\)',line);count=re.search(r'text_bytes=(\d+)',line)
  if request and count:counts[int(request[1])]=int(count[1])
assert counts[after['id']]-counts[before['id']]==len('new pronunciation marker')-len('before'),counts
probe.save(root/'result.json',dict(success=True,before_text_bytes=counts[before['id']],after_text_bytes=counts[after['id']],applied=applied));print(counts)
