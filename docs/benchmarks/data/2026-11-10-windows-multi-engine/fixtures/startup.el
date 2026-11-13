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
(princ (format "Startup implementation %s\n" (symbol-file 'tts-make-process 'defun)))
(defvar startup--spans nil)
(defvar startup--depth 0)
(dolist (name '(tts-make-process omnivox-library--prepare-engines omnivox-library--owner
                omnivox-library--acknowledge-engines omnivox-library--request omnivox-library--control
                omnivox-library--retire tts--retire-process voice-setup make-process
                omnivox-native-guard--prepare-bridge omnivox-native-guard--connect omnivox-native-guard--request))
  (let ((function name))
    (advice-add name :around
                (lambda (original &rest arguments)
                  (let ((start (float-time)) (depth startup--depth) (startup--depth (1+ startup--depth)))
                    (unwind-protect (apply original arguments)
                      (push (list :function (symbol-name function) :depth depth :start start
                                  :ms (* 1000 (- (float-time) start))) startup--spans)))))))
(let ((records nil) (engine (or (getenv "OMNIVOX_ENGINE") "flite")))
  (unwind-protect
      (progn
        (let ((start (float-time)))
          (tts-initialize)
          (let ((ready (float-time)) (speech (emergency-test--speech engine)))
            (push (list :operation "initialize" :ready_ms (* 1000 (- ready start))
                        :first_ms (* 1000 (- (plist-get speech :first) start))) records)))
        (dotimes (trial 4)
          (let* ((previous (emergency-test--speech engine))
                 (start (float-time)))
            (tts-restart)
            (let ((ready (float-time)) (speech (emergency-test--speech engine)))
              (push (list :operation "restart" :trial trial
                          :operation_ms (* 1000 (- ready start))
                          :first_ms (* 1000 (- (plist-get speech :first) start))
                          :source_gap_bound_ms (* 1000 (- (plist-get speech :first) (plist-get previous :completed)))) records))))
        (with-temp-file (expand-file-name "startup-results.json" (getenv "OMNIVOX_TEST_CONFIG_DIR"))
          (insert (json-serialize (vconcat (reverse records)))))
        (with-temp-file (expand-file-name "startup-spans.json" (getenv "OMNIVOX_TEST_CONFIG_DIR"))
          (insert (json-serialize (vconcat (reverse startup--spans)))))
        (dolist (name '("Speaker" "Notify" "Native guard"))
          (when-let* ((buffer (tts--diagnostic-buffer name)))
            (with-current-buffer buffer
              (write-region (point-min) (point-max)
                            (expand-file-name (concat name ".stderr") (getenv "OMNIVOX_TEST_CONFIG_DIR")) nil 'silent))))
        (princ (format "STARTUP RESULTS %S\n" (reverse records))))
    (dolist (process (list tts-speaker-process tts-notify-process))
      (when (processp process) (tts--retire-process process)))))
