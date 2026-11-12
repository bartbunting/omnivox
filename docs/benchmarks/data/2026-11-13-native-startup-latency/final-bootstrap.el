;;; evox-windows-init.el --- Bart's native Windows profile -*- lexical-binding: t; -*-

(unless (and (eq system-type 'windows-nt) (version<= "30.2" emacs-version))
  (error "Native Windows Emacs 30.2+ required"))

(defun bb-native-personal-check ()
  "Check personal configuration and both native speech lanes in a fresh GUI."
  (condition-case problem
      (progn
        (unless (and (eq window-system 'w32)
                     (bound-and-true-p vertico-mode)
                     (bound-and-true-p global-corfu-mode)
                     (equal user-full-name "Bart Bunting")
                     (eq (key-binding (kbd "C-c a")) #'org-agenda)
                     (eq (key-binding (kbd "C-c e")) #'my/open-in-windows-explorer)
                     (eq emacsvox-sounds-current-pack 'bart)
                     (file-executable-p auth-source-1password-executable))
          (error "Personal settings incomplete: %S"
                 (list window-system (bound-and-true-p vertico-mode)
                       (bound-and-true-p global-corfu-mode) user-full-name
                       (key-binding (kbd "C-c a")) (key-binding (kbd "C-c e"))
                       emacsvox-sounds-current-theme auth-source-1password-executable)))
        (dolist (lane `((,tts-speaker-process . "Your native Windows customisations are ready.")
                        (,tts-notify-process . "Your notification speech is ready.")))
          (let ((deadline (+ (float-time) 40)))
            (while (and (< (float-time) deadline)
                        (not (process-get (car lane) omnivox--control-registration-property)))
              (accept-process-output nil 0.05)))
          (unless (and (process-live-p (car lane))
                       (process-get (car lane) omnivox--control-registration-property))
            (error "Speech lane is not ready"))
          (let ((tts-speaker-process (car lane)) status
                (deadline (+ (float-time) 30)))
            (tts--protocol-queue-text (cdr lane))
            (tts--protocol-dispatch-tracked (lambda (_ value) (setq status value)))
            (while (and (not status) (< (float-time) deadline))
              (accept-process-output nil 0.05))
            (unless (eq status 'completed) (error "Speech failed: %s" status))))
        (with-temp-file (getenv "BB_EVOX_CHECK")
          (insert "PASS: native GUI, shared personal settings, completion, Org and Explorer bindings, Bart sound pack, Windows 1Password path, and both speech lanes.\n"))
        (kill-emacs 0))
    (error
     (with-temp-file (getenv "BB_EVOX_CHECK")
       (insert (error-message-string problem)))
     (kill-emacs 1))))

(condition-case problem
    (let* ((personal (file-name-as-directory (getenv "EMACSVOX_PERSONAL_DIRECTORY")))
           (configuration (expand-file-name "config/" personal)))
      (setq user-emacs-directory (expand-file-name "profile/" personal)
            default-directory (file-name-as-directory (getenv "USERPROFILE")))
      (make-directory user-emacs-directory t)
      (load (expand-file-name "early-init.el" configuration) nil 'nomessage)
      (modify-frame-parameters
       nil (mapcar (lambda (key) (assq key default-frame-alist))
                   '(foreground-color background-color fullscreen)))
      (load (expand-file-name "init.el" configuration) nil 'nomessage)
      (setq frame-title-format '("Emacsvox Windows — " (:eval (buffer-name))))
      (when (getenv "BB_EVOX_CHECK")
        (add-hook 'emacs-startup-hook
                  (lambda () (run-at-time 2 nil #'bb-native-personal-check)))))
  (error
   (when (getenv "BB_EVOX_CHECK")
     (with-temp-file (getenv "BB_EVOX_CHECK")
       (insert (error-message-string problem)))
     (kill-emacs 1))
   (signal (car problem) (cdr problem))))

;;; evox-windows-init.el ends here
