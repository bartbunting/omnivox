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



(require 'omnivox-preferences)
(let (records)
  (unwind-protect
      (progn
        (tts-initialize)
        (emergency-test--speech "flite")
        (let ((old tts-speaker-process) (peer tts-notify-process))
          (dotimes (trial 4)
            (let ((pending (make-hash-table :test #'eq)) samples reloads failures timer prepared begin end)
              (unwind-protect
                  (progn
                    (setq begin (float-time))
                    (dolist (lane (list old peer))
                      (omnivox-preferences--reload-worker
                       lane (if (eq lane old) 'main 'notification) nil
                       (lambda (result) (push (plist-get result :outcome) reloads))))
                    (setq timer
                          (run-at-time
                           0 .015
                           (lambda ()
                             (condition-case err
                                 (dolist (lane (list old peer))
                                   (unless (gethash lane pending)
                                     (puthash lane t pending)
                                     (let ((tts-speaker-process lane)
                                           (row (list :lane (if (eq lane old) "main" "notification")
                                                      :submitted (float-time) :first nil :terminal nil)))
                                       (push row samples)
                                       (tts-speak-marked
                                        "A"
                                        (lambda (_id event)
                                          (when (equal (plist-get event :type) "utterance_started")
                                            (setf (plist-get row :first) (float-time))))
                                        (lambda (_id status)
                                          (setf (plist-get row :terminal) (symbol-name status))
                                          (remhash lane pending))))))
                               (error (push (error-message-string err) failures))))))
                    (setq prepared (omnivox-library--prepare-emergency (tts--resolve-program tts-program) "null"))
                    (setq end (float-time))
                    (cancel-timer timer)
                    (emergency-test--wait (lambda () (and (zerop (hash-table-count pending)) (= 2 (length reloads)))))
                    (unless (and (not failures) (eq tts-speaker-process old) (eq tts-notify-process peer)
                                 (cl-every (lambda (v) (member v '("applied" "unchanged"))) reloads)
                                 (cl-every (lambda (row) (equal (plist-get row :terminal) "completed")) samples))
                      (error "Preparation disturbed old speech: %S %S %S" failures reloads samples))
                    (dolist (name '("main" "notification"))
                      (unless (cl-some (lambda (row) (and (equal name (plist-get row :lane))
                                                         (plist-get row :first) (< (plist-get row :first) end))) samples)
                        (error "No source speech observed during preparation for %s" name)))
                    (push (list :trial trial :prepare_ms (* 1000 (- end begin)) :reloads (vconcat reloads)
                                :samples (vconcat (reverse samples))) records)
                    (princ (format "PASS preparation %d: %d completed short utterances, both lanes spoke during preparation\n" trial (length samples))))
                (when (timerp timer) (cancel-timer timer))
                (when prepared (omnivox-library--discard-emergency prepared)))))))
    (dolist (process (delete-dups (append (list tts-speaker-process tts-notify-process omnivox-library--emergency-pending)
                                         omnivox-native-guard--owners)))
      (when (processp process) (omnivox-library--retire-for-emergency process))))
  (with-temp-file (expand-file-name "availability.json" (getenv "OMNIVOX_TEST_CONFIG_DIR"))
    (insert (json-serialize (vconcat (nreverse records))))))
