;;; paths.el --- Local WSL resources for this Windows test -*- lexical-binding: t; -*-
(require 'subr-x)
(defvar omnivox-listening--paths (make-hash-table :test #'equal))
(defun omnivox-listening--windows-resource (path)
  "Translate a local WSL resource PATH for this profile's Windows worker."
  (if (not (string-prefix-p "/" path)) path
    (or (gethash path omnivox-listening--paths)
        (let ((native
               (with-temp-buffer
                 (unless (zerop (process-file "wslpath" nil t nil "-w" path))
                   (error "Cannot translate speech resource for Windows"))
                 (string-trim (buffer-string)))))
          (when (>= (hash-table-count omnivox-listening--paths) 256)
            (clrhash omnivox-listening--paths))
          (puthash path native omnivox-listening--paths)))))
(with-eval-after-load 'omnivox-remote
  (advice-add 'omnivox-remote-resource :filter-return
              #'omnivox-listening--windows-resource))
