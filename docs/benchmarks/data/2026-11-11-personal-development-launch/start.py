#!/usr/bin/env python3
"""Local Development launcher: qualified speech with Bart's normal Emacs profile."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
SOURCE = Path('/home/bart/src/emacsvox-lightweight-reload')
EMACS = '/home/bart/opt/emacs-31/bin/emacs'
HOME = Path('/home/bart')
PROFILE = HOME / '.emacsvox.d'
QUALIFIED = HERE.parent / '2026-11-10-early-speech-refresh/listening'
ENGINE_ROOT = Path('/mnt/c/Users/bart/AppData/Local/Emacsvox/Omnivox/profiles/development-lightweight-reload')


def prepare(arguments):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check-launch', action='store_true', help='Validate and describe the launch without opening Emacs')
    parser.add_argument('--terminal', action='store_true')
    parser.add_argument('--engine', choices=('espeak', 'flite', 'winrt', 'eloquence', 'dectalk', 'rutts', 'tgspeechbox', 'rhvoice'), default='eloquence')
    options, extra = parser.parse_known_args(arguments)
    manifest = json.loads((QUALIFIED / 'runtime.json').read_text())
    executable = Path(manifest['executable'])
    if hashlib.sha256(executable.read_bytes()).hexdigest() != manifest['sha256']:
        raise SystemExit('Qualified speech executable changed; check its qualification first.')
    for file in (PROFILE / 'early-init.el', PROFILE / 'init.el',
                 HOME / 'emacs/site-lisp/emacsvox-config.org', HERE / 'bootstrap.el'):
        if not file.is_file():
            raise SystemExit(f'Missing profile input: {file}')
    subprocess.run(['make', 'bytecode-check', f'EMACS={EMACS}'], cwd=SOURCE, check=True)
    ENGINE_ROOT.mkdir(parents=True, exist_ok=True)
    config = ENGINE_ROOT / 'config.json'
    # Seed only the private engine setup; subsequent settings persist unchanged.
    try:
        with config.open('x') as output:
            json.dump({'schema': 4, 'engine_overrides': {key: {'enabled': False}
                      for key in ('macos', 'piper', 'mbrola')}}, output, indent=2)
            output.write('\n')
    except FileExistsError:
        pass

    def windows(path):
        return subprocess.check_output(['wslpath', '-w', str(path)], text=True).strip()

    environment = dict(os.environ)
    for key in list(environment):
        if key.startswith(('OMNIVOX_', 'SWIFTMAC_', 'SHARPWIN_')):
            environment.pop(key)
    environment.update(
        HOME=str(HOME), XDG_CONFIG_HOME=str(HOME / '.config'),
        XDG_STATE_HOME=str(HOME / '.local/state'), XDG_CACHE_HOME=str(HOME / '.cache'),
        EMACSVOX_DIR=str(SOURCE), EMACSVOX_INIT_DIRECTORY=str(PROFILE),
        EMACSVOX_CONFIG_FILE=str(HOME / 'emacs/site-lisp/emacsvox-config.org'),
        EMACSVOX_PACKAGE_DIRECTORY=str(HOME / '.emacs.d/elpa'),
        TTS_PROGRAM=str(SOURCE / 'servers/omnivox'),
        OMNIVOX_PROGRAM=str(executable), OMNIVOX_CONFIG_DIR=windows(ENGINE_ROOT),
        OMNIVOX_VOICE_ROOT=windows(ENGINE_ROOT / 'voices'), OMNIVOX_ENGINE=options.engine,
        OMNIVOX_AUDIO_OUTPUT='device', OMNIVOX_EXPERIMENTAL_EARLY_SPEECH='1',
        ESPEAK_NG_DATA=manifest['data'], OMNIVOX_RHVOICE_LIBRARY=manifest['rhvoice_library'],
        OMNIVOX_RHVOICE_DATA=manifest['rhvoice_data'],
        OMNIVOX_LOG_DIRECTORY=str(HOME / '.local/state/emacsvox/development-omnivox'),
        EMACSVOX_PLAY=str(SOURCE / 'servers/windows-play'))
    # The ordinary server wrapper exports its other native settings itself.
    wslenv = [entry for entry in environment.get('WSLENV', '').split(':')
              if entry and entry.split('/')[0] != 'OMNIVOX_EXPERIMENTAL_EARLY_SPEECH']
    environment['WSLENV'] = ':'.join(wslenv + ['OMNIVOX_EXPERIMENTAL_EARLY_SPEECH'])
    command = [EMACS, f'--init-directory={PROFILE}', '-q', '--debug-init', '--no-splash',
               '--name', 'emacsvox-development']
    if options.terminal:
        command.append('-nw')
    command += ['--load', str(PROFILE / 'early-init.el'), '--load', str(HERE / 'bootstrap.el'),
                '--load', str(PROFILE / 'init.el'), '--eval',
                '(setq frame-title-format (quote ("Emacsvox Development — " (:eval (buffer-name)))))']
    command += extra
    return options, command, environment


def main():
    # Automated recovery checks keep the qualified disposable profile; they
    # must never edit the personal preferences or create its Emacs server.
    if any(flag in sys.argv[1:] for flag in ('--batch-check', '--gui-check')):
        os.execv(sys.executable, [sys.executable, str(QUALIFIED / 'start.py'), *sys.argv[1:]])
    options, command, environment = prepare(sys.argv[1:])
    if options.check_launch:
        print(json.dumps({'command': command, 'environment': {key: environment[key] for key in
              ('HOME', 'EMACSVOX_DIR', 'EMACSVOX_CONFIG_FILE', 'TTS_PROGRAM', 'OMNIVOX_PROGRAM',
               'OMNIVOX_CONFIG_DIR', 'OMNIVOX_ENGINE', 'OMNIVOX_EXPERIMENTAL_EARLY_SPEECH')}}, indent=2))
        return
    print(f'Emacs profile: {PROFILE}\nSpeech executable: {environment["OMNIVOX_PROGRAM"]}', flush=True)
    os.execve(command[0], command, environment)


if __name__ == '__main__':
    main()
