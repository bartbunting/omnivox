;;; check.el --- Full interactive isolated startup check -*- lexical-binding: t; -*-
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
              (with-temp-file (expand-file-name "gui-check.json" root)
                (insert (json-serialize
                         (list :result "passed" :engine expected
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
