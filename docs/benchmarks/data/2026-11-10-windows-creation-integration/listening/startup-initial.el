;;; startup.el --- Isolated guarded speech profile -*- lexical-binding: t; -*-
(let ((source (file-name-as-directory (getenv "EMACSVOX_DIR")))
      (profile (file-name-as-directory (getenv "OMNIVOX_LISTENING_ROOT"))))
  (setq user-emacs-directory (expand-file-name "emacs/" profile)
        custom-file (expand-file-name "custom.el" user-emacs-directory)
        load-prefer-newer nil
        omnivox-native-guard--enabled t
        tts-notification-device "both")
  (add-to-list 'load-path (expand-file-name "lisp/" source))
  (require 'emacsvox-preamble)
  (setq emacsvox-user-directory (expand-file-name "resources/" profile))
  (load (expand-file-name "lisp/emacsvox-setup.el" source) nil nil t)
  (unless (and (string-prefix-p source (symbol-file 'tts-initialize 'defun))
               (string-suffix-p ".elc" (symbol-file 'tts-initialize 'defun)))
    (error "The isolated client was not loaded"))
  (when noninteractive
    (tts-initialize)
    (let ((deadline (+ (float-time) 15)))
      (while (and (< (float-time) deadline)
                  (not (and (process-live-p tts-speaker-process)
                            (process-live-p tts-notify-process))))
        (accept-process-output nil .01)))
    (unless (and (process-live-p tts-speaker-process)
                 (process-live-p tts-notify-process))
      (error "The isolated speech lanes did not start"))
    (let ((deadline (+ (float-time) 15)))
      (while (and (< (float-time) deadline)
                  (not (condition-case nil
                           (omnivox--choice-current-registration tts-speaker-process)
                         (error nil))))
        (accept-process-output nil .01)))
    (let ((deadline (+ (float-time) 15)) completed actual)
      (tts-speak-marked
       "The isolated speech prototype is ready."
       (lambda (_id record)
         (when (equal (plist-get record :type) "utterance_started")
           (setq actual (plist-get record :engine_id))))
       (lambda (_id status) (setq completed status)))
      (while (and (not completed) (< (float-time) deadline))
        (accept-process-output nil .01))
      (unless (and (eq completed 'completed)
                   (equal actual (getenv "OMNIVOX_LISTENING_ENGINE")))
        (error "Speech failed: %S %S" completed actual))
      (princ (format "Speech source completed using %s.\n" actual)))
    (dolist (process (list tts-speaker-process tts-notify-process))
      (unless (and (process-live-p (process-get process 'omnivox-native-guard))
                   (equal "windows-pipe"
                          (plist-get (process-get process 'omnivox-native-guard-endpoint)
                                     :transport)))
        (error "A speech lane is missing its Windows guard")))
    (princ (format "Client: %s\nProfile: %s\n" (symbol-file 'tts-initialize 'defun) profile)))
  (unless noninteractive
    (switch-to-buffer (get-buffer-create "*Omnivox guarded prototype*"))
    (insert "Omnivox guarded Windows prototype\n\n"
            "This profile has separate settings and speech processes.\n"
            "Only eSpeak and Flite are qualified for this initial test.\n\n"
            "Navigate this text by character and word; interrupt a longer line.\n"
            "Try the normal speech settings controls.\n"
            "M-x omnivox-reload-preferences applies saved host preferences.\n"
            "M-x tts-restart restarts speech with newly saved settings.\n"
            "M-x tts-recover restores main speech using its retained startup.\n"
            "M-x tts-emergency-start starts minimal eSpeak after verified cleanup.\n\n"
            "The quick brown fox jumps over the lazy dog.\n"
            "Punctuation: alpha, beta; gamma! Capital letters: A B C.\n")
    (goto-char (point-min))
    (text-mode)))
