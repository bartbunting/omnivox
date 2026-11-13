#!/usr/bin/env python3
"""Fixture forwarding owner; all four descriptors remain owned until I/O ends."""
import os
import sys
import threading

speech_input, speech_output, lifetime, completion = map(int, sys.argv[1:])
for fd in (speech_input, speech_output, lifetime, completion):
    os.set_inheritable(fd, False)


def copy(source, destination):
    try:
        while data := os.read(source, 65536):
            while data:
                data = data[os.write(destination, data):]
    except BrokenPipeError:
        pass


output = threading.Thread(target=copy, args=(speech_output, 1))
output.start()
copy(0, speech_input)
os.close(speech_input)
output.join()
os.close(speech_output)
os.close(lifetime)
os.close(completion)  # Only after both I/O owners finish; death also closes all.
