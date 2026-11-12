;;; -*- lexical-binding: t; -*-
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
      (evox-timing-mark "registration-ack" (vector (process-name process) generation))))
  (advice-add 'omnivox--dispatch-playback-marker-event :before
    (lambda (&rest _) nil)))
(with-eval-after-load 'tts-lifecycle
  (advice-add 'tts--observe-dispatch-source :before
    (lambda (process _identifier event)
      (when (equal (plist-get event :type) "utterance_started")
        (evox-timing-mark "speech-source" (vector (process-name process) (plist-get event :engine_id)))))))
(add-hook 'emacs-startup-hook (lambda () (evox-timing-mark "startup-hook")))
(add-hook 'kill-emacs-hook
  (lambda ()
    (evox-timing-mark "exit")
    (with-temp-file (expand-file-name (concat (getenv "EVOX_TIMING_RUN") "-emacs.json") (getenv "EVOX_TIMING_ROOT"))
      (insert (json-serialize (vconcat (nreverse evox-timing-events)))))))
(condition-case err
    (load (expand-file-name "bin/evox-windows-init.el" (getenv "USERPROFILE")) nil t)
  (error
   (evox-timing-mark "profile-error" (error-message-string err))
   (kill-emacs 2)))
(evox-timing-mark "profile-loaded")
