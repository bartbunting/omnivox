from pathlib import Path
stage=Path('/mnt/c/Users/bart/AppData/Local/Emacsvox/NativeLightweight/native-early-20261113')
p=stage/'startup-timing-20261113'
base=(stage/'evox-windows.ps1').read_text()
base=base.replace('Set-StrictMode -Version Latest', '''$timing = New-Object Collections.Generic.List[object]
function Mark([string]$Event) { $timing.Add(@{event=$Event; epoch=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()/1000.0}) }
Mark 'powershell-script'
Set-StrictMode -Version Latest''')
base=base.replace('$selected = Get-', "Mark 'version-start'\n$selected = Get-")
base=base.replace("if (-not (Test-Path -LiteralPath $config.Omnivox))", "Mark 'version-end'\nif (-not (Test-Path -LiteralPath $config.Omnivox))")
base=base.replace("$arguments = @", "Mark 'preflight-end'\n$arguments = @")
base=base.replace('Invoke-EmacsvoxNative $selected.Program @(\'-Q\', \'--batch\'', "Mark 'preflight-start'\nInvoke-EmacsvoxNative $selected.Program @('-Q', '--batch'")
base=base.replace('(Get-EmacsvoxLoadExpression $startup))', "(Get-EmacsvoxLoadExpression (Join-Path $PSScriptRoot 'trace.el')))")
base=base[:base.index('if ($Check) {')]+'''$result = Join-Path $PSScriptRoot ($env:EVOX_TIMING_RUN + '-check.json')
$environment.EMACSVOX_NATIVE_PERSONAL_CHECK = '1'
$environment.EMACSVOX_NATIVE_PROBE_RESULT = $result
$environment.EVOX_TIMING_ROOT = $PSScriptRoot
$environment.XDG_STATE_HOME = Join-Path $PSScriptRoot ($env:EVOX_TIMING_RUN + '-state')
$arguments += @('--eval', (Get-EmacsvoxLoadExpression (Join-Path $PSScriptRoot 'finish.el')))
Mark 'gui-start'
try {
    Invoke-EmacsvoxNative $selected.Program $arguments $environment (Join-Path $PSScriptRoot ($env:EVOX_TIMING_RUN + '.log')) 180 | Out-Null
} finally {
    Mark 'gui-exit'
    $timing | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $PSScriptRoot ($env:EVOX_TIMING_RUN + '-launcher.json'))
}
'''
(p/'measure.ps1').write_text(base)
(p/'trace.el').write_text(''';;; -*- lexical-binding: t; -*-
(defvar evox-timing-events nil)
(defun evox-timing-mark (event &optional detail)
  (push (list :epoch (float-time) :event event :detail detail) evox-timing-events))
(evox-timing-mark "elisp-start")
(add-hook 'after-load-functions
  (lambda (file) (evox-timing-mark "loaded" (and file (file-name-nondirectory file)))))
(defun evox-timing-around (name original &rest args)
  (evox-timing-mark (concat name "-start"))
  (unwind-protect (apply original args) (evox-timing-mark (concat name "-end"))))
(dolist (entry '((package package-initialize)
                 (ob-tangle org-babel-load-file)
                 (tts-speak tts-initialize)
                 (emacsvox emacsvox emacsvox--restore-startup-presentation)
                 (omnivox-native-guard omnivox-native-guard-launch)
                 (comp comp-subr-trampoline-install)))
  (eval-after-load (car entry)
    (lambda ()
      (dolist (fn (cdr entry))
        (when (fboundp fn)
          (advice-add fn :around (apply-partially #'evox-timing-around (symbol-name fn))))))))
(with-eval-after-load 'omnivox-voices
  (advice-add 'omnivox--accept-registration-response :before
    (lambda (process _response generation _content)
      (evox-timing-mark "registration-ack" (list (process-name process) generation))))
  (advice-add 'omnivox--dispatch-playback-marker-event :before
    (lambda (&rest _) nil)))
(with-eval-after-load 'tts-lifecycle
  (advice-add 'tts--observe-dispatch-source :before
    (lambda (process _identifier event)
      (when (equal (plist-get event :type) "utterance_started")
        (evox-timing-mark "speech-source" (list (process-name process) (plist-get event :engine_id)))))))
(add-hook 'emacs-startup-hook (lambda () (evox-timing-mark "startup-hook")))
(add-hook 'kill-emacs-hook
  (lambda ()
    (evox-timing-mark "exit")
    (with-temp-file (expand-file-name (concat (getenv "EVOX_TIMING_RUN") "-emacs.json") (getenv "EVOX_TIMING_ROOT"))
      (insert (json-serialize (vconcat (nreverse evox-timing-events)))))))
(load (expand-file-name "bin/evox-windows-init.el" (getenv "USERPROFILE")) nil t)
(evox-timing-mark "profile-loaded")
''')
(p/'finish.el').write_text(''';;; -*- lexical-binding: t; -*-
(evox-timing-mark "check-start")
(load (expand-file-name "../native-check.el" (getenv "EVOX_TIMING_ROOT")) nil t)
''')
