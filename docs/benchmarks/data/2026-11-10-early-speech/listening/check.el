;;; check.el --- Full interactive isolated startup check -*- lexical-binding: t; -*-
(require 'cl-lib)
(defun omnivox-startup-check-sound (expected)
  (let ((speak (symbol-function 'tts-speak-marked))
        (resource (symbol-function 'omnivox-remote-resource))
        mapped)
    (cl-letf (((symbol-function 'tts-speak-marked)
               (lambda (text marker completion)
                 (funcall speak (propertize text 'auditory-icon 'warn-user)
                          marker completion)))
              ((symbol-function 'omnivox-remote-resource)
               (lambda (path)
                 (let ((native (funcall resource path)))
                   (when (string-suffix-p "warn-user.ogg" path)
                     (push native mapped))
                   native))))
      (omnivox-listening--check expected))
    (unless (and mapped
                 (cl-every (lambda (path) (string-prefix-p "\\\\" path)) mapped))
      (error "The completed speech did not include a mapped Windows sound: %S" mapped))))

(defun omnivox-startup-check ()
  (let ((root (file-name-as-directory (getenv "OMNIVOX_LISTENING_ROOT")))
        (expected (getenv "OMNIVOX_LISTENING_ENGINE")) failure)
    (unwind-protect
        (condition-case err
            (progn
              (dolist (file (list emacsvox-aural-schemes-file
                                 emacsvox-aural-routing-profiles-file
                                 user-emacs-directory custom-file))
                (unless (file-in-directory-p file root)
                  (error "Profile path escapes isolation: %s" file)))
              (omnivox-listening--check expected)
              (let ((tts-speaker-process tts-notify-process))
                (omnivox-listening--check expected))
              (omnivox-startup-check-sound expected)
              (let ((tts-speaker-process tts-notify-process))
                (omnivox-startup-check-sound expected))
              (with-temp-file (expand-file-name "gui-check.json" root)
                (insert (json-serialize
                         (list :result "passed" :engine expected :sound_and_speech "completed on both lanes"
                               :client (symbol-file 'omnivox-apply-voice-configuration 'defun)
                               :schemes emacsvox-aural-schemes-file
                               :routing emacsvox-aural-routing-profiles-file)))))
          (error (setq failure (error-message-string err))))
      (dolist (process (delete-dups (append (list tts-speaker-process tts-notify-process)
                                           omnivox-native-guard--owners)))
        (when (processp process)
          (condition-case err (omnivox-library--retire-for-emergency process)
            (error (setq failure (error-message-string err)))))))
    (when failure
      (with-temp-file (expand-file-name "gui-check-error.txt" root) (insert failure)))
    (kill-emacs (if failure 1 0))))
;; Run as the next command-line form on the UI thread. Timers can execute in
;; a startup background thread, which cannot await the UI-owned guard bridge.
(omnivox-startup-check)
