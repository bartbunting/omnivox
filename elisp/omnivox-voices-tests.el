;;; omnivox-voices-tests.el --- Omnivox adapter tests -*- lexical-binding: t; -*-

;;; Commentary:

;; Exercise the standalone Emacspeak compatibility adapter without requiring
;; an Emacspeak installation.

;;; Code:

(require 'cl-lib)
(require 'ert)

(defvar dtk-program "")
(defvar dtk-speaker-process nil)
(defvar dtk-speech-rate 50)
(defvar dtk-speech-rate-step 5)

(provide 'emacspeak-preamble)
(load
 (expand-file-name
  "omnivox-voices.el"
  (file-name-directory (or load-file-name buffer-file-name)))
 nil nil)

(ert-deftest omnivox-emacspeak-rate-steps-follow-server-scale ()
  "Faster raises the server rate while slower lowers it."
  (let ((dtk-speech-rate 50)
        (dtk-speech-rate-step 5)
        requested)
    (cl-letf (((symbol-function 'omnivox-set-rate)
               (lambda (rate) (push rate requested))))
      (omnivox-faster)
      (omnivox-slower))
    (should (equal (nreverse requested) '(55 45)))))

(ert-deftest omnivox-emacspeak-rate-clamps-advertised-range ()
  "Rate commands send only values in the advertised zero-to-100 range."
  (let ((original-rate (default-value 'omnivox-speech-rate))
        (original-dtk-rate (default-value 'dtk-speech-rate))
        writes)
    (unwind-protect
        (cl-letf (((symbol-function 'omnivox--send)
                   (lambda (command) (push command writes)))
                  ((symbol-function 'message) #'ignore))
          (omnivox-set-rate 105)
          (should (= (default-value 'omnivox-speech-rate) 100))
          (should (= (default-value 'dtk-speech-rate) 100))
          (omnivox-set-rate -5)
          (should (= (default-value 'omnivox-speech-rate) 0))
          (should (= (default-value 'dtk-speech-rate) 0))
          (should
           (equal
            (nreverse writes)
            '("tts_set_speech_rate 100" "tts_set_speech_rate 0"))))
      (set-default 'omnivox-speech-rate original-rate)
      (set-default 'dtk-speech-rate original-dtk-rate))))

(ert-run-tests-batch-and-exit)

;;; omnivox-voices-tests.el ends here
