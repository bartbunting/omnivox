;;; run-emergency-speech.el --- Isolated minimal recovery probe -*- lexical-binding: t; -*-
;; Copyright (C) 2026 Emacsvox contributors
;; SPDX-License-Identifier: GPL-2.0-or-later

(require 'cl-lib)
(defvar emergency-test--root
  (file-name-as-directory
   (or (getenv "OMNIVOX_TEST_SOURCE_ROOT")
       (expand-file-name "../" (file-name-directory load-file-name)))))
(add-to-list 'load-path (expand-file-name "lisp/" emergency-test--root))
(setq load-prefer-newer (not (getenv "OMNIVOX_TEST_BYTECODE")))
(require 'emacsvox-preamble)
(require 'tts-speak)
(require 'omnivox-library-runtime)
(require 'omnivox-voices)
(unless (getenv "OMNIVOX_TEST_BYTECODE")
  (dolist (file '("omnivox-native-guard.el" "tts-speak.el" "tts-lifecycle.el" "omnivox-library-runtime.el" "omnivox-voices.el"))
    (load file nil nil)))
(when-let* ((legacy (getenv "OMNIVOX_TEST_LEGACY_ROOT")))
  (load (expand-file-name "omnivox-library-runtime.elc" legacy) nil nil t)
  (load (expand-file-name "omnivox-native-guard.elc" legacy) nil nil t))
(setq omnivox-native-guard--enabled t)
(setq emacsvox-servers-directory (expand-file-name "servers/" emergency-test--root)
      tts-program "omnivox" tts-notification-device "both" emacsvox-speak-messages nil
      debug-on-error t)
(dolist (function '(tts-emergency-start tts--recover-speaker omnivox-library--retire-for-emergency omnivox-native-guard--retire))
  (let ((file (symbol-file function 'defun)))
    (unless (and (string-prefix-p (if (memq function '(omnivox-library--retire-for-emergency omnivox-native-guard--retire))
                                          (or (getenv "OMNIVOX_TEST_LEGACY_ROOT") emergency-test--root)
                                        emergency-test--root) file)
                 (string-suffix-p (if (getenv "OMNIVOX_TEST_BYTECODE") ".elc" ".el") file))
      (error "Wrong implementation %S: %S" function file))
    (princ (format "Loaded %s from %s\n" function file))))
(unless (and (getenv "OMNIVOX_PROGRAM") (getenv "OMNIVOX_TEST_CONFIG_DIR")
             (getenv "OMNIVOX_TEST_VOICE_ROOT"))
  (error "Explicit isolated executable, configuration and voice root required"))

(defun emergency-test--wait (predicate)
  (let ((deadline (+ (float-time) 10)))
    (while (and (not (funcall predicate)) (< (float-time) deadline))
      (accept-process-output nil .005))
    (unless (funcall predicate) (error "Emergency observation timed out"))))

(defun emergency-test--speech (engine)
  (emergency-test--wait
   (lambda () (and (condition-case nil (omnivox--choice-current-registration tts-speaker-process)
                    (error nil))
                  (cl-every
                   (lambda (process)
                     (let ((pending (process-get process omnivox--control-pending-property)))
                       (or (not pending) (= 0 (hash-table-count pending)))))
                   (list tts-speaker-process tts-notify-process)))))
  (let ((start (float-time)) terminal actual first)
    (tts-speak-marked "Emergency speech works."
                      (lambda (_id record)
                        (when (equal (plist-get record :type) "utterance_started")
                          (setq actual (plist-get record :engine_id))
                          (unless first (setq first (float-time)))))
                      (lambda (_id status) (setq terminal status)))
    (emergency-test--wait (lambda () terminal))
    (unless (and (eq terminal 'completed) (equal actual engine))
      (error "Wrong speech result: %S %S" terminal actual))
    (princ (format "Speech %s source %.3f ms, completion %.3f ms\n"
                   engine (* 1000 (- first start)) (* 1000 (- (float-time) start))))
    (list :submitted start :first first :completed (float-time))))

(defun emergency-test--fingerprints (root)
  (mapcar (lambda (file)
            (cons file (with-temp-buffer (insert-file-contents-literally file)
                                         (secure-hash 'sha256 (current-buffer)))))
          (directory-files-recursively root ".")))




(when-let* ((baseline (getenv "OMNIVOX_TEST_STARTUP_BASELINE")))
  (load baseline nil nil t))
(require 'omnivox-preferences)
(when-let* ((delay (getenv "OMNIVOX_TEST_NOTIFICATION_DELAY")))
  (advice-add 'tts-notify-initialize :before
              (lambda (&rest _)
                (let ((until (+ (float-time) (string-to-number delay))))
                  (while (< (float-time) until) (accept-process-output nil .005))))))
(defvar availability--retirements nil)
(advice-add 'tts--retire-process :before
            (lambda (p) (push (list :process (process-name p) :time (float-time)) availability--retirements)))
(let (records)
  (unwind-protect
      (progn
        (tts-initialize)
        (emergency-test--speech "flite")
        (dotimes (trial 4)
          (let ((old tts-speaker-process) (peer tts-notify-process)
                (pending (make-hash-table :test #'eq)) samples failures timer begin end)
            (setq availability--retirements nil)
            (unwind-protect
                (progn
                  (setq timer
                        (run-at-time
                         0 .025
                         (lambda ()
                           (dolist (pair (list (cons "main" tts-speaker-process) (cons "notification" tts-notify-process)))
                             (let ((lane (cdr pair)) (name (car pair)))
                               (when (and (process-live-p lane)
                                          (not (gethash lane pending))
                                          (not (process-get lane 'tts--speech-process-retiring))
                                          (not (process-get lane 'omnivox-library-retired))
                                          (condition-case nil (omnivox--choice-current-registration lane) (error nil)))
                                 (condition-case err
                                     (let ((tts-speaker-process lane)
                                           (row (list :lane name :process (process-name lane)
                                                      :old (if (memq lane (list old peer)) t :false)
                                                      :submitted (float-time) :first nil :terminal nil)))
                                       (puthash lane t pending)
                                       (push row samples)
                                       (tts-speak-marked
                                        "A"
                                        (lambda (_id event)
                                          (when (equal (plist-get event :type) "utterance_started")
                                            (setf (plist-get row :first) (float-time))))
                                        (lambda (_id status)
                                          (setf (plist-get row :terminal) (symbol-name status))
                                          (remhash lane pending))))
                                   (error (remhash lane pending) (push (error-message-string err) failures)))))))))
                  (let ((until (+ (float-time) .2))) (while (< (float-time) until) (accept-process-output nil .01)))
                  (setq begin (float-time))
                  (tts-restart)
                  (setq end (float-time))
                  (let ((until (+ (float-time) .5))) (while (< (float-time) until) (accept-process-output nil .01)))
                  (cancel-timer timer)
                  (emergency-test--wait (lambda () (zerop (hash-table-count pending))))
                  (with-temp-file (expand-file-name (format "availability-trial-%s.json" trial) (getenv "OMNIVOX_TEST_CONFIG_DIR"))
                    (insert (json-serialize (list :start begin :end end :samples (vconcat (reverse samples)) :failures (vconcat failures)))))
                  (unless (and (not failures)
                               (cl-every (lambda (r) (member (plist-get r :terminal) '("completed" "cancelled"))) samples))
                    (error "Restart speech failed: %S %S" failures samples))
                  (dolist (name '("main" "notification"))
                    (unless (and
                             (cl-some (lambda (r) (and (equal name (plist-get r :lane))
                                                       (eq t (plist-get r :old)) (plist-get r :first)
                                                       (> (plist-get r :first) begin) (< (plist-get r :first) end))) samples)
                             (cl-some (lambda (r) (and (equal name (plist-get r :lane))
                                                       (eq :false (plist-get r :old)) (plist-get r :first))) samples))
                      (error "Missing old/new source speech for %s" name)))
                  (push (list :trial trial :start begin :end end :operation_ms (* 1000 (- end begin))
                              :samples (vconcat (reverse samples))
                              :retirements (vconcat (reverse availability--retirements))) records)
                  (princ (format "PASS restart %d: %d requests across old and new lanes\n" trial (length samples))))
              (when (timerp timer) (cancel-timer timer))))))
    (dolist (process (list tts-speaker-process tts-notify-process))
      (when (processp process) (tts--retire-process process))))
  (with-temp-file (expand-file-name "restart-availability.json" (getenv "OMNIVOX_TEST_CONFIG_DIR"))
    (insert (json-serialize (vconcat (reverse records))))))
