#!/usr/bin/env python3
"""Kill only the pidfd-bound worker below this isolated Emacs launcher."""
import os,signal,sys
from pathlib import Path
root=int(sys.argv[1]);exe=Path(sys.argv[2]).resolve();todo=[root];seen=set();owners=[]
def children(pid):return Path(f'/proc/{pid}/task/{pid}/children').read_text().split()
while todo:
    pid=todo.pop();assert len(seen)<128,'unexpected fixture tree size'
    if pid in seen:continue
    seen.add(pid)
    try:
        args=Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0')
        if Path(f'/proc/{pid}/exe').resolve()==exe and b'--voice-library-owner' in args:owners.append(pid)
        todo.extend(map(int,children(pid)))
    except FileNotFoundError:pass
assert len(owners)==1,owners
owner=owners[0];owner_handle=os.pidfd_open(owner)
try:
    ids=[int(pid) for pid in children(owner) if Path(f'/proc/{pid}/exe').resolve()==exe]
    assert len(ids)==1,ids
    worker=ids[0];handle=os.pidfd_open(worker)
    try:
        assert str(worker) in children(owner)
        assert Path(f'/proc/{worker}/exe').resolve()==exe
        signal.pidfd_send_signal(handle,signal.SIGKILL)
        print('Killed isolated worker',worker,'owned by',owner,'below launcher',root)
    finally:os.close(handle)
finally:os.close(owner_handle)
