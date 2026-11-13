;;; bootstrap.el --- Qualified speech for the personal Development profile -*- lexical-binding: t; -*-
;; Local launcher bridge; the personal init still owns settings and integrations.
(require 'subr-x)
(setq omnivox-native-guard--enabled t)
(startup-redirect-eln-cache "/home/bart/.cache/emacsvox-lightweight-test/eln-cache")
(defvar omnivox-development--paths (make-hash-table :test #'equal))
(defun omnivox-development--windows-resource (path)
  "Translate a local WSL resource PATH for this profile's Windows worker."
  (if (not (string-prefix-p "/" path)) path
    (or (gethash path omnivox-development--paths)
        (let ((native
               (with-temp-buffer
                 (unless (zerop (process-file "wslpath" nil t nil "-w" path))
                   (error "Cannot translate speech resource for Windows"))
                 (string-trim (buffer-string)))))
          (when (>= (hash-table-count omnivox-development--paths) 256)
            (clrhash omnivox-development--paths))
          (puthash path native omnivox-development--paths)))))
(with-eval-after-load 'omnivox-remote
  (advice-add 'omnivox-remote-resource :filter-return
              #'omnivox-development--windows-resource))
