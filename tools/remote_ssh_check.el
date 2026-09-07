;;; remote_ssh_check.el --- Real SSH acceptance -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Omnivox contributors
;; SPDX-License-Identifier: GPL-2.0-or-later

;;; Commentary:
;; Invoked only by check_remote_ssh.py in a private remote source snapshot.
;; The harness owns the service and deliberately interrupts its SSH forward.

;;; Code:

(setq load-prefer-newer t)
(let ((root (getenv "EMACSVOX_DIR")))
  (add-to-list 'load-path (expand-file-name "lisp" root))
  (load (expand-file-name "lisp/emacsvox-preamble.el" root) nil t))
(require 'cl-lib)
(require 'tts-speak)
(require 'omnivox-voices)

(setq tts-program "omnivox"
      tts-notification-device nil
      omnivox-remote-host "127.0.0.1"
      omnivox-remote-port (string-to-number (getenv "OMNIVOX_REMOTE_TEST_PORT"))
      omnivox-remote-token-file (getenv "OMNIVOX_REMOTE_TEST_TOKEN")
      omnivox-remote-auto-reconnect t)

(defun omnivox-ssh-check-wait (predicate description &optional seconds)
  "Wait for PREDICATE for SECONDS, reporting DESCRIPTION on timeout."
  (let ((deadline (+ (float-time) (or seconds 30))))
    (while (and (not (funcall predicate)) (< (float-time) deadline))
      (accept-process-output nil 0.05))
    (unless (funcall predicate) (error "Timed out: %s" description))))

(defun omnivox-ssh-check-ready (process)
  "Whether PROCESS has negotiated inventory, voices, and initial routing."
  (and (process-live-p process)
       (process-get process omnivox--control-inventory-property)
       (process-get process omnivox--control-registration-property)
       (omnivox--process-routing-policy-current-p process)))

(defun omnivox-ssh-check-speech (process text)
  "Verify TEXT completes on PROCESS using the requested engine."
  (let ((tts-speaker-process process)
        (tts-stop-immediately nil)
        terminals engines)
    (tts-speak-marked
     text
     (lambda (_ event)
       (when (equal "utterance_started" (plist-get event :type))
         (push (plist-get event :engine_id) engines)))
     (lambda (_ status) (push status terminals)))
    (omnivox-ssh-check-wait (lambda () terminals) "marked speech" 15)
    (unless (equal terminals '(completed))
      (error "Speech terminal statuses: %S" terminals))
    (unless (and engines
                 (cl-every (lambda (engine)
                             (equal engine (getenv "OMNIVOX_REMOTE_TEST_ENGINE")))
                           engines))
      (error "Unexpected realized engines: %S" engines))))

(condition-case problem
    (unwind-protect
        (cl-letf (((symbol-function 'tts--resolve-program)
                   (lambda (&rest _) (error "Remote Emacs attempted a local launch"))))
          (message "Remote Emacs %s" emacs-version)
          (omnivox-remote-connect)
          (omnivox-ssh-check-wait
           (lambda () (cl-every #'omnivox-ssh-check-ready
                               (list tts-speaker-process tts-notify-process)))
           "both lane inventories, registrations, and routing")
          (unless (omnivox-query-voices) (error "Workstation has no voices"))
          (omnivox-ssh-check-speech tts-speaker-process "Remote foreground is ready.")
          (omnivox-ssh-check-speech tts-notify-process "Remote notification is ready.")
          ;; Remain idle longer than the service's 20-second lease.  Only the
          ;; client's normal heartbeat should keep these same workers alive.
          (let ((old (list tts-speaker-process tts-notify-process))
                (until (+ (float-time) 22)))
            (while (< (float-time) until) (accept-process-output nil 0.1))
            (unless (and (equal old (list tts-speaker-process tts-notify-process))
                         (cl-every #'omnivox-ssh-check-ready old))
              (error "Idle heartbeat did not preserve both lanes")))
          (message "OMNIVOX-SSH-CHECK idle-passed")
          (let ((old-speaker tts-speaker-process)
                (old-notify tts-notify-process)
                speaker-status notify-status)
            ;; Long pending work makes accidental queue replay observable:
            ;; fresh speech must finish in 15 seconds after reconnection.
            ;; Null output consumes silence without a wall-clock delay, so
            ;; include enough synthesis to remain pending across an SSH RTT.
            (process-send-string
             old-speaker
             (concat "sh 60000\nq "
                     (apply #'concat (make-list 4000 "Obsolete foreground speech. ")) "\n"))
            (tts--protocol-dispatch-tracked
             (lambda (_ status) (push status speaker-status)))
            (let ((tts-speaker-process old-notify))
              (process-send-string
               old-notify
               (concat "sh 60000\nq "
                       (apply #'concat (make-list 4000 "Obsolete notification speech. ")) "\n"))
              (tts--protocol-dispatch-tracked
               (lambda (_ status) (push status notify-status))))
            (when (or speaker-status notify-status)
              (error "Speech finished before the interruption was armed"))
            ;; Batch stdout can be buffered through SSH.  `message' writes
            ;; the synchronization records immediately to stderr.
            (message "OMNIVOX-SSH-CHECK interrupt-now")
            (omnivox-ssh-check-wait
             (lambda () (and speaker-status notify-status)) "lost dispatch failures")
            (unless (and (equal speaker-status '(failed))
                         (equal notify-status '(failed)))
              (error "Interrupted request statuses: %S / %S" speaker-status notify-status))
            (message "OMNIVOX-SSH-CHECK disconnected")
            (omnivox-ssh-check-wait
             (lambda ()
               (and (not (eq old-speaker tts-speaker-process))
                    (not (eq old-notify tts-notify-process))
                    (omnivox-ssh-check-ready tts-speaker-process)
                    (omnivox-ssh-check-ready tts-notify-process)))
             "automatic recovery of both lanes" 45)
            (omnivox-ssh-check-speech tts-speaker-process "Foreground recovered.")
            (omnivox-ssh-check-speech tts-notify-process "Notification recovered.")
            (unless (and (equal speaker-status '(failed))
                         (equal notify-status '(failed)))
              (error "An interrupted request received a second terminal status")))
          (message "OMNIVOX-SSH-CHECK passed"))
      (omnivox-remote-disconnect))
  (error
   ;; Avoid batch Lisp backtraces, which can contain authentication records.
   (princ (format "SSH acceptance failed: %s\n" (error-message-string problem))
          'external-debugging-output)
   (kill-emacs 1)))

;;; remote_ssh_check.el ends here
