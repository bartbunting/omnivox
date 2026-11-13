;;; profile.el --- Private refresh cost observations -*- lexical-binding: t; -*-
(defvar discovery-profile-rows nil)
(dolist (fn '(process-file call-process call-process-region load))
  (advice-add fn :around
              (lambda (old &rest args)
                (let ((start (float-time)))
                  (unwind-protect (apply old args)
                    (push (list fn start (* 1000 (- (float-time) start)) (mapcar (lambda (x) (if (stringp x) x (format "%S" x))) args)) discovery-profile-rows))))))
(with-eval-after-load 'omnivox-library-runtime
  (dolist (fn '(omnivox-library--wait omnivox-library--prepare-engines
                omnivox-library--remember-engines omnivox-library--wait-for-recovered-speech
                tts-notify-initialize tts-make-process))
    (when (fboundp fn)
      (advice-add fn :around
                  (lambda (old &rest args)
                    (let ((start (float-time)))
                      (unwind-protect (apply old args)
                        (push (list fn start (* 1000 (- (float-time) start))) discovery-profile-rows))))))))
(with-eval-after-load 'omnivox-voices
  (dolist (fn '(omnivox--handle-discovery-response omnivox--handle-inventory-response
                omnivox--register-logical-voices omnivox--prepare-voice-registrations
                omnivox--accept-registration-response omnivox--control-process-filter
                omnivox--refresh-discovery))
    (when (fboundp fn)
      (advice-add fn :around
                  (lambda (old &rest args)
                    (let ((start (float-time)))
                      (unwind-protect (apply old args)
                        (push (list fn start (* 1000 (- (float-time) start))) discovery-profile-rows))))))))
(add-hook 'kill-emacs-hook
          (lambda ()
            (with-temp-file (expand-file-name "refresh-profile.el" (getenv "OMNIVOX_LISTENING_ROOT"))
              (prin1 (reverse discovery-profile-rows) (current-buffer)))))
