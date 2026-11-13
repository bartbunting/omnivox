#!/usr/bin/env python3
"""Matched guarded/direct source timing during per-worker reload; no acoustic claim."""
import concurrent.futures, hashlib, json, os, select, statistics, subprocess, sys, time, threading
from pathlib import Path
sys.dont_write_bytecode=True
repo=next(p for p in Path(__file__).resolve().parents if (p/'Cargo.toml').is_file())
sys.path.insert(0,str(repo/'docs/benchmarks/data/2026-11-10-emergency-speech'))
import probe as emergency
resident=emergency.resident
class Peer(emergency.Peer):
 def rpc(self,kind,**fields):
  if kind != 'reload_punctuation_v1':return super().rpc(kind,**fields)
  with self.write_lock:
   self.control+=1;identity=self.control
   self.p.stdin.write(b'omnivox_control '+resident.encoded(dict(protocol_version=1,request_id=identity,type=kind,**fields))+b'\n');self.p.stdin.flush()
  self.reload_submitted.set()
  return self.wait(lambda r:r['kind']=='control' and r['value'].get('request_id')==identity)['value']
 def local(self,command,**fields):
  value=super().local(command,**fields)
  if command=='describe' and value.get('guard') and not hasattr(self,'bridge'):
   ep=value['guard'];self.ep=ep;self.guard_seq=0
   self.bridge=subprocess.Popen([self.p.args[0],'--native-guard-control'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.stderr,bufsize=0)
   self.bridge.stdin.write(json.dumps(ep).encode()+b'\n');self.guard('attach')
   assert super().local('start-guarded',worker=value['worker'])['type']=='started'
  return value
 def guard(self,command):
  self.guard_seq+=1;ep=self.ep
  self.bridge.stdin.write(json.dumps(dict(command=command,guard=ep['guard'],worker=ep['worker'],token=ep['token'],request_id=self.guard_seq)).encode()+b'\n')
  assert select.select([self.bridge.stdout],[],[],6)[0];r=json.loads(self.bridge.stdout.readline());assert not r['error'];return r
 def close(self):
  if hasattr(self,'bridge'):
   self.retire();self.bridge.stdin.close();self.bridge.wait(timeout=5);self.bridge.stdout.close()
  return super().close()
