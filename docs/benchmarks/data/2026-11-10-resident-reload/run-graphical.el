;; Isolated graphical ERT runner; never loads a live profile.
(require 'ert)
(require 'omnivox-preferences-tests)
(let ((log "/tmp/emacsvox-resident-graphical-2.log") (status 2)
      (debug-on-error nil) (debug-on-quit nil))
  (advice-add 'message :after
              (lambda (format-string &rest arguments)
                (when format-string
                  (with-temp-buffer
                    (insert (apply #'format-message format-string arguments) "\n")
                    (write-region (point-min) (point-max) log t 'silent)))))
  (condition-case error-data
      (progn
        (unless (display-graphic-p) (error "No graphical frame"))
        (let ((stats (ert-run-tests-batch "omnivox-.*graphical")))
          (setq status (if (and (= 2 (ert-stats-completed stats))
                               (= 0 (ert-stats-completed-unexpected stats))
                               (= 0 (ert-stats-skipped stats))) 0 1))))
    (error (message "Graphical runner failed: %S" error-data)))
  (kill-emacs status))
