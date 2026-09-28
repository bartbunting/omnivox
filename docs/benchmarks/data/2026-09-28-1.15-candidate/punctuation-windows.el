;;; -*- lexical-binding: t; -*-
;; Qualification through the actual bundled WSL launcher and native service.
(require 'ert)
(require 'cl-lib)
(load "/home/bart/src/emacsvox/lisp/emacsvox-preamble.el" nil nil)
(require 'omnivox-punctuation)
(require 'tts-speak)
(ert-deftest punctuation-windows-launcher-round-trip ()
  (let* ((root (make-temp-file "/mnt/c/Users/bart/AppData/Local/Temp/omnivox punctuation editor " t))
         (file (expand-file-name "config.json" root))
         (process-environment (copy-sequence process-environment))
         (tts-program "omnivox")
         (emacsvox-servers-directory "/home/bart/src/emacsvox/servers/")
         (load-path load-path))
    (unwind-protect
        (progn
          (setenv "OMNIVOX_CONFIG_DIR"
                  (with-temp-buffer
                    (call-process "wslpath" nil t nil "-w" root)
                    (string-trim (buffer-string))))
          (setenv "OMNIVOX_VOICE_ROOT"
                  (with-temp-buffer
                    (call-process "wslpath" nil t nil "-w" (expand-file-name "voices" root))
                    (string-trim (buffer-string))))
          (with-temp-file file
            (insert "{\"schema\":2,\"speech\":{\"defaults\":{\"rate\":0.7}}}"))
          (cl-letf (((symbol-function 'emacsvox-aural-ui-speak) #'ignore)
                    ((symbol-function 'tts-restart) (lambda () (ert-fail "Save restarted speech"))))
            (with-temp-buffer
              (omnivox-punctuation-mode)
              (unwind-protect
                  (progn
                    (omnivox-punctuation--accept (omnivox-punctuation--request '(:command "punctuation-review")))
                    (should (string-suffix-p (concat (getenv "OMNIVOX_CONFIG_DIR") "\\config.json") (plist-get omnivox-punctuation--review :path)))
                    (omnivox-punctuation--change "’" "Speak a name" "single quote")
                    (omnivox-punctuation--change "$" "Preserve character")
                    (omnivox-punctuation-save)
                    (should-not (omnivox-punctuation--dirty-p))
                    (let* ((review (omnivox-punctuation--request '(:command "punctuation-review")))
                           (tables (omnivox-punctuation--tables (plist-get review :overrides))))
                      (should (equal (gethash "’" (cdr (assoc "some" tables))) "single quote"))
                      (should (eq (gethash "$" (cdr (assoc "some" tables))) :null)))
                    (omnivox-punctuation--change "’" "Speak a name" "changed draft")
                    (with-temp-buffer
                      (insert-file-contents file)
                      (goto-char (point-max))
                      (insert "\n")
                      (write-region (point-min) (point-max) file nil 'silent))
                    (should-error (omnivox-punctuation-save))
                    (should (omnivox-punctuation--dirty-p)))
                (setq kill-buffer-query-functions nil))))
          (with-temp-buffer
            (insert-file-contents file)
            (let ((data (json-parse-buffer :object-type 'plist)))
              (should (= (plist-get data :schema) 3))
              (should (= (plist-get (plist-get (plist-get data :speech) :defaults) :rate) 0.7)))))
      (delete-directory root t))))
(ert-run-tests-batch-and-exit "punctuation-windows-launcher-round-trip")
