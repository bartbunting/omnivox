#!/usr/bin/env python3
import os,sys
args=sys.argv[1:]
if args and args[0]=='run':
    args[1:1]=['--volume','/home/bart/src/omnivox/.git:/home/bart/src/omnivox/.git:ro']
os.execv('/usr/bin/docker',['docker']+args)
