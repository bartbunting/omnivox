;;; probe.el -*- lexical-binding: t; -*-
(defun ov-probe-log (tag data)
  (let ((print-length 12) (print-level 5))
    (with-temp-buffer
      (insert (format "%s %s %S\n" (float-time) tag data))
      (write-region (point-min) (point-max) (getenv "OV_PROBE_LOG") t 'silent))))
(with-eval-after-load 'omnivox-voices
  (when (getenv "OV_FORCE_TIMEOUT")
    (setq omnivox-voice-configuration-timeout 0.2)
    (advice-add 'omnivox--handle-control-line :around
      (lambda (fn process line)
        (let ((response (and (string-prefix-p omnivox-control-event-prefix line)
                             (ignore-errors (omnivox--decode-control-response (substring line (length omnivox-control-event-prefix)))))))
          (if (and (equal (plist-get response :type) "logical_voices_registered_v3")
                   (> (or (plist-get response :registry_generation) 0) 1))
              (progn (ov-probe-log 'delayed (list (process-name process) (plist-get response :request_id)))
                     (run-at-time 0.5 nil fn process line) t)
            (funcall fn process line))))))
  (advice-add 'omnivox--dispatch-control-response :before
    (lambda (process response)
      (ov-probe-log 'receive (list (process-name process) (plist-get response :type) (plist-get response :request_id) (plist-get response :registry_generation)))))
  (advice-add 'omnivox--send-control-request :before
    (lambda (process request &rest _)
      (ov-probe-log 'send (list (process-name process) (plist-get request :type)
                                (plist-get request :registry_generation)))))
  (advice-add 'omnivox--accept-registration-response :around
    (lambda (fn process response generation content)
      (let ((before (plist-get (process-get process 'omnivox--choice-registration) :registry-generation))
            (result (funcall fn process response generation content)))
        (ov-probe-log 'ack (list (process-name process) (plist-get response :type)
                                (plist-get response :request_id)
                                generation (plist-get response :registry_generation)
                                :before before :accepted result
                                :after (plist-get (process-get process 'omnivox--choice-registration) :registry-generation)))
        result)))
  (advice-add 'omnivox--record-control-error :before
    (lambda (process response) (ov-probe-log 'error (list (process-name process) response)))))
(defun ov-probe-finish ()
  (condition-case err
      (progn
        (ov-probe-log 'state (list :error omnivox-control-last-error :marker-error omnivox-marker-last-error :apply omnivox-voice-configuration-last-result :pending (hash-table-count (omnivox--pending-requests tts-speaker-process))))
        (ov-probe-log 'files (mapcar (lambda (s) (list s (and (boundp s) (symbol-value s))))
                            '(emacsvox-user-directory emacsvox-aural-routing-profiles-file
                              emacsvox-aural-schemes-file)))
        (dotimes (i 4)
          (let ((deadline (+ (float-time) 4)) status)
            (tts-speak-marked "Foreground probe." (lambda (&rest _) nil)
                              (lambda (_id result) (setq status result)))
            (while (and (not status) (< (float-time) deadline)) (accept-process-output nil .01))
            (ov-probe-log 'speech (list i status)))))
    (error (ov-probe-log 'probe-error err)))
  (when (get-buffer "*Messages*")
    (with-current-buffer "*Messages*"
      (write-region (point-min) (point-max) (concat (getenv "OV_PROBE_LOG") ".messages") nil 'silent)))
  (dolist (process (delete-dups (append (list tts-speaker-process tts-notify-process)
                                       omnivox-native-guard--owners)))
    (when (processp process) (ignore-errors (omnivox-library--retire-for-emergency process))))
  (kill-emacs 0))
(add-hook 'emacs-startup-hook (lambda () (run-at-time 3 nil #'ov-probe-finish)))
