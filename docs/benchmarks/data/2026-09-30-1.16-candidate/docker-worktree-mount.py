#!/usr/bin/python3
import os,sys
args=sys.argv[1:]
if args and args[0]=='run':
    args[1:1]=['--mount','type=bind,source=/home/bart/src/omnivox/.git,target=/home/bart/src/omnivox/.git,readonly']
os.execv('/usr/bin/docker', ['/usr/bin/docker']+args)
