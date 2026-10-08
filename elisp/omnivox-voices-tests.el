;;; omnivox-voices-tests.el --- Omnivox adapter tests -*- lexical-binding: t; -*-

;;; Commentary:

;; Exercise the standalone Emacspeak compatibility adapter without requiring
;; an Emacspeak installation.

;;; Code:

(require 'cl-lib)
(require 'ert)

(cl-defstruct acss family average-pitch pitch-range stress richness)

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

(ert-deftest omnivox-control-request-preserves-unicode-on-one-line ()
  (let* ((request '(:protocol_version 1 :request_id 42 :type "preview"
                   :text "Zażółć gęślą\njaźń."
                   :selector (:kind "engine_default" :engine_id "espeak")))
         (encoded (omnivox--encode-control-request request)))
    (should-not (string-match-p "[\r\n]" encoded))
    (should (equal request
                   (json-parse-string
                    (decode-coding-string (base64-decode-string encoded) 'utf-8)
                    :object-type 'plist)))))

(ert-deftest omnivox-control-response-preserves-json-values ()
  (let ((response
         (omnivox--decode-control-response
          (base64-encode-string
           "{\"request_id\":0,\"enabled\":false,\"voice\":null,\"features\":[]}"
           t))))
    (should (equal response
                   '(:request_id 0 :enabled :false :voice :null
                     :features nil)))))

(ert-deftest omnivox-control-rejects-invalid-and-oversized-payloads ()
  (should-error (omnivox--decode-control-response "!not-base64!"))
  (should-error (omnivox--decode-control-response "ew==")) ; Incomplete JSON: {
  (let ((omnivox--control-max-payload-bytes 16))
    ;; This JSON fits in 16 characters, but exceeds 16 UTF-8 bytes.
    (should-error (omnivox--encode-control-request '(:text "żółć")))
    (should-error (omnivox--decode-control-response (make-string 28 ?A)))
    ;; Seventeen decoded bytes still fit in the 24-byte encoded limit.
    (should-error
     (omnivox--decode-control-response
      (base64-encode-string (concat "\"" (make-string 15 ?A) "\"") t)))))

(defmacro omnivox-test--with-process (name &rest body)
  (declare (indent 1))
  `(let ((,name (make-pipe-process :name "omnivox-test" :noquery t)))
     (unwind-protect (progn ,@body)
       (delete-process ,name))))

(defun omnivox-test--control-event (id &optional type version)
  (concat "__OMNIVOX_CONTROL__ "
          (omnivox--encode-control-request
           (list :protocol_version (or version 1) :request_id id
                 :type (or type "capabilities") :message "test reply"))
          "\n"))

(ert-deftest omnivox-control-request-handles-fragments-and-process-local-ids ()
  (omnivox-test--with-process main
    (omnivox-test--with-process notification
      (let* (chunks forwarded
             (filter (lambda (_process text) (push text forwarded))))
        (dolist (process (list main notification))
          (set-process-filter process filter))
        (cl-letf
            (((symbol-function 'process-send-string)
              (lambda (process wire)
                (should (string-prefix-p "omnivox_control {" wire))
                (should (string-suffix-p "}\n" wire))
                (should-error
                 (omnivox--control-request process '(:type "capabilities")))
                (let* ((request (omnivox--decode-control-response
                                 (substring wire 17 -2)))
                       (reply (concat "diagnostic\n"
                                      (omnivox-test--control-event 0)
                                      (omnivox-test--control-event
                                       (plist-get request :request_id)))))
                  (setq chunks (list (substring reply 0 13)
                                     (substring reply 13 -1)
                                     (substring reply -1))))))
             ((symbol-function 'accept-process-output)
              (lambda (process &rest _)
                (funcall (process-filter process) process (pop chunks)))))
          (dolist (case (list (cons main 1) (cons main 2)
                             (cons notification 1)))
            (let ((reply (omnivox--control-request
                          (car case) '(:type "capabilities"))))
              (should (= (plist-get reply :request_id) (cdr case)))
              (should (eq (process-filter (car case)) filter))
              (should-not (process-get (car case) 'omnivox--control-busy)))))
        (let ((expected (concat "diagnostic\n"
                                (omnivox-test--control-event 0))))
          (should (equal forwarded (make-list 3 expected))))))))

(ert-deftest omnivox-control-request-restores-filter-after-failure ()
  (dolist (output (list ""
                       "__OMNIVOX_CONTROL__ invalid!\n"
                       (omnivox-test--control-event 1 "error")
                       (omnivox-test--control-event 1 "capabilities" 2)
                       (make-string
                        (1+ (* 2 omnivox--control-max-payload-bytes)) ?A)))
    (omnivox-test--with-process process
      (set-process-filter process #'ignore)
      (cl-letf (((symbol-function 'process-send-string)
                 (lambda (owner _wire)
                   (funcall (process-filter owner) owner output))))
        (should-error
         (omnivox--control-request process '(:type "capabilities") 0.01)))
      (should (eq (process-filter process) #'ignore))
      (should-not (process-get process 'omnivox--control-busy)))))

(defconst omnivox-test--capabilities
  '(:type "capabilities" :supported_protocol_versions (1)
    :features ("logical_voice_registration" "logical_voice_routing")))

(ert-deftest omnivox-logical-voice-registration-is-process-local ()
  (omnivox-test--with-process main
    (omnivox-test--with-process notification
      (let ((definitions [(:id "keyword" :preferences []
                           :acss (:pitch_range 0.25))])
            (bindings '((:status "unresolved" :error
                         (:logical_voice_id "keyword"))))
            requests)
        (cl-letf (((symbol-function 'omnivox--control-request)
                   (lambda (process request)
                     (push (cons process request) requests)
                     (if (equal (plist-get request :type) "capabilities")
                         omnivox-test--capabilities
                       (list :type "logical_voices_registered"
                             :registration
                             (list :registry_generation
                                   (plist-get request :registry_generation)
                                   :bindings bindings))))))
          (dolist (process (list main main notification))
            (should (equal (plist-get (omnivox--register-logical-voices
                                      process definitions) :bindings)
                           bindings))))
        (setq requests (nreverse requests))
        (should (equal (mapcar (lambda (entry) (plist-get (cdr entry) :type))
                              requests)
                       '("capabilities" "register_logical_voices"
                         "register_logical_voices" "capabilities"
                         "register_logical_voices")))
        (dolist (case (list (list 1 main 1) (list 2 main 2)
                           (list 4 notification 1)))
          (let ((entry (nth (car case) requests)))
            (should (eq (car entry) (nth 1 case)))
            (should (= (plist-get (cdr entry) :registry_generation)
                       (nth 2 case)))
            (should (equal (plist-get (cdr entry) :definitions)
                           definitions))))))))

(ert-deftest omnivox-logical-voice-registration-requires-capabilities ()
  (dolist (reply '((:type "unexpected")
                   (:type "capabilities" :supported_protocol_versions (2))
                   (:type "capabilities" :supported_protocol_versions (1)
                    :features ("logical_voice_registration"))
                   (:type "capabilities" :supported_protocol_versions (1)
                    :features ("logical_voice_routing"))))
    (omnivox-test--with-process process
      (cl-letf (((symbol-function 'omnivox--control-request)
                 (lambda (_process request)
                   (should (equal request '(:type "capabilities")))
                   reply)))
        (should-error (omnivox--register-logical-voices process nil)))
      (should-not (process-get process 'omnivox--registry-generation)))))

(ert-deftest omnivox-logical-voice-registration-validates-replies ()
  (omnivox-test--with-process process
    (let ((attempt 0))
      (cl-letf (((symbol-function 'omnivox--control-request)
                 (lambda (_process request)
                   (if (equal (plist-get request :type) "capabilities")
                       omnivox-test--capabilities
                     (cl-incf attempt)
                     (should (= (plist-get request :registry_generation)
                                attempt))
                     (should (equal (plist-get request :definitions) []))
                     (pcase attempt
                       (1 (error "Omnivox control request timed out"))
                       (2 '(:type "logical_voices_registered"
                            :registration (:registry_generation 1)))
                       (3 '(:type "unexpected"
                            :registration (:registry_generation 3)))
                       (_ (list :type "logical_voices_registered"
                                :registration
                                (list :registry_generation attempt
                                      :bindings nil))))))))
        (dotimes (_ 3)
          (should-error (omnivox--register-logical-voices process nil)))
        (should (equal (omnivox--register-logical-voices process nil)
                       '(:registry_generation 4 :bindings nil)))))))

(ert-deftest omnivox-espeak-definition-preserves-identity-and-empty-style ()
  (let* ((style (make-acss :stress 8))
         (definition (omnivox--espeak-voice-definition
                      'overlay style "espeak:zlw/pl+m3")))
    (should (equal definition
                   '(:id "overlay"
                     :preferences [(:kind "exact" :engine_id "espeak"
                                    :voice_id "espeak:zlw/pl+m3")]
                     :acss nil)))
    (should (string-match-p "\"acss\":{}" (json-serialize definition)))
    (should (equal style (make-acss :stress 8)))
    (should-error
     (omnivox--espeak-voice-definition 'overlay style "macos:voice"))))

(ert-deftest omnivox-espeak-definition-maps-standard-styles ()
  (dolist (case
           (list
            (list (make-acss :average-pitch 4 :stress 6)
                  (list :average_pitch (/ 4.0 9))) ; voice-bolden
            (list (make-acss :pitch-range 8 :stress 8 :richness 8)
                  '(:pitch_range 0.8 :volume 0.45)) ; voice-animate-extra
            (list (make-acss :pitch-range 0 :stress 0)
                  '(:pitch_range 0.0)) ; voice-monotone-extra
            (list (make-acss :stress 0 :richness 2)
                  '(:volume 0.15)) ; voice-smoothen-extra
            (list (make-acss :average-pitch 0 :pitch-range 0 :richness 0)
                  '(:average_pitch 0.0 :pitch_range 0.0 :volume 0.05))
            (list (make-acss :average-pitch 9 :pitch-range 9 :richness 9)
                  '(:average_pitch 1.0 :pitch_range 0.9 :volume 0.5))
            (list (make-acss :pitch-range 2) '(:pitch_range 0.17))
            (list (make-acss :pitch-range 4) '(:pitch_range 0.37))))
    (should (equal (plist-get (omnivox--espeak-voice-definition
                              'style (car case) "espeak:zlw/pl") :acss)
                   (cadr case)))))

(ert-deftest omnivox-espeak-definition-rejects-invalid-levels ()
  (dolist (dimension '(:average-pitch :pitch-range :richness))
    (dolist (value '(-1 10 0.5 "5"))
      (should-error
       (omnivox--espeak-voice-definition
        'invalid (apply #'make-acss (list dimension value))
        "espeak:zlw/pl")))))

(defmacro omnivox-test--with-styles (&rest body)
  (declare (indent 0))
  `(let ((omnivox-voice-table (make-hash-table))
         (omnivox--acss-styles (make-hash-table))
         (omnivox-voice-id "espeak:zlw/pl")
         (omnivox-voice-string "[[pitch 1]]"))
     (omnivox-define-voice 'paul omnivox-voice-string)
     ,@body))

(ert-deftest omnivox-styles-register-once-for-each-process ()
  (omnivox-test--with-styles
    (omnivox-test--with-process main
      (omnivox-test--with-process notification
        (omnivox-test--with-process restarted
          (let (registrations)
            (cl-letf (((symbol-function 'omnivox--register-logical-voices)
                       (lambda (process definitions)
                         (push (cons process definitions) registrations))))
              (omnivox-define-voice-from-acss
               'monotone (make-acss :pitch-range 0))
              (omnivox-define-voice-from-acss
               'bold (make-acss :average-pitch 4))
              (should-not registrations)
              (dolist (dtk-speaker-process
                       (list main main notification restarted))
                (should (equal (omnivox-get-voice-command 'monotone)
                               "[[logical_voice monotone]] [[pitch 1]]"))
                (should (equal (omnivox-get-voice-command 'paul)
                               "[[pitch 1]] [{voice espeak:zlw/pl}]"))))
            (should (equal (mapcar #'car (reverse registrations))
                           (list main notification restarted)))
            (dolist (entry registrations)
              (should (equal (sort (mapcar (lambda (definition)
                                            (plist-get definition :id))
                                          (cdr entry)) #'string<)
                             '("bold" "monotone"))))))))))

(ert-deftest omnivox-styles-resync-after-style-and-voice-changes ()
  (omnivox-test--with-styles
    (omnivox-test--with-process dtk-speaker-process
      (let (registrations)
        (cl-letf (((symbol-function 'omnivox--register-logical-voices)
                   (lambda (_process definitions)
                     (push definitions registrations))))
          (omnivox-define-voice-from-acss 'style (make-acss :pitch-range 0))
          (omnivox-get-voice-command 'style)
          (omnivox-define-voice-from-acss 'style (make-acss :pitch-range 8))
          (omnivox-get-voice-command 'style)
          (setq omnivox-voice-id "espeak:zlw/pl+m3")
          (omnivox-get-voice-command 'style))
        (should (= (length registrations) 3))
        (should (equal (mapcar (lambda (definitions)
                                (plist-get (plist-get (elt definitions 0) :acss)
                                           :pitch_range))
                              (reverse registrations))
                       '(0.0 0.8 0.8)))
        (should (equal (plist-get
                        (aref (plist-get (elt (car registrations) 0)
                                         :preferences) 0)
                        :voice_id)
                       "espeak:zlw/pl+m3"))))))

(ert-deftest omnivox-styles-compare-definition-content ()
  (omnivox-test--with-styles
    (omnivox-test--with-process dtk-speaker-process
      (let (registrations)
        (cl-letf (((symbol-function 'omnivox--register-logical-voices)
                   (lambda (_process definitions)
                     (push definitions registrations))))
          (omnivox-define-voice-from-acss 'first (make-acss :pitch-range 0))
          (omnivox-define-voice-from-acss 'second (make-acss :richness 6))
          (omnivox-get-voice-command 'first)
          (omnivox-define-voice-from-acss 'first (make-acss :pitch-range 0))
          (omnivox-get-voice-command 'first)
          (should (= (length registrations) 1))
          ;; Rebuilding the table in reverse order changes no definitions.
          (setq omnivox--acss-styles (make-hash-table))
          (omnivox-define-voice-from-acss 'second (make-acss :richness 6))
          (omnivox-define-voice-from-acss 'first (make-acss :pitch-range 0))
          (omnivox-get-voice-command 'first)
          (should (= (length registrations) 1))
          (setf (acss-pitch-range (gethash 'first omnivox--acss-styles)) 8)
          (omnivox-get-voice-command 'first)
          (should (= (length registrations) 2))
          (should (= (plist-get (plist-get (aref (car registrations) 0) :acss)
                                :pitch_range) 0.8))
          (omnivox-define-voice 'second "[[pitch 1.2]]")
          (omnivox-get-voice-command 'first)
          (should (= (length registrations) 3))
          (should (= (length (car registrations)) 1)))))))

(ert-deftest omnivox-styles-respect-manual-voices-and-other-engines ()
  (omnivox-test--with-styles
    (omnivox-test--with-process dtk-speaker-process
      (omnivox-define-voice-from-acss 'style (make-acss :pitch-range 0))
      (cl-letf (((symbol-function 'omnivox--register-logical-voices) #'ignore))
        (omnivox-get-voice-command 'style))
      (let ((manual "[{voice macos:custom}] [[pitch 1.2]]"))
        (omnivox-define-voice 'style manual)
        (should-not (gethash 'style omnivox--acss-styles))
        (cl-letf (((symbol-function 'omnivox--sync-espeak-styles)
                   (lambda (&rest _) (ert-fail "Unexpected registration"))))
          (should (equal (omnivox-get-voice-command 'style)
                         (concat manual " [{voice espeak:zlw/pl}]"))))))
    (omnivox-test--with-process dtk-speaker-process
      (let ((omnivox-voice-id "macos:default"))
        (omnivox-define-voice-from-acss 'style (make-acss :average-pitch 4))
        (cl-letf (((symbol-function 'omnivox--sync-espeak-styles)
                   (lambda (&rest _) (ert-fail "Unexpected registration"))))
          (should (equal (omnivox-get-voice-command 'style)
                         (gethash 'style omnivox-voice-table))))))))

(ert-deftest omnivox-styles-fall-back-without-retrying-every-utterance ()
  (omnivox-test--with-styles
    (omnivox-test--with-process dtk-speaker-process
      (let ((attempts 0) warnings)
        (omnivox-define-voice-from-acss 'style (make-acss :average-pitch 4))
        (cl-letf (((symbol-function 'omnivox--register-logical-voices)
                   (lambda (&rest _)
                     (cl-incf attempts)
                     (error "Unsupported control protocol")))
                  ((symbol-function 'display-warning)
                   (lambda (_type message &rest _) (push message warnings))))
          (dotimes (_ 2)
            (should (equal (omnivox-get-voice-command 'style)
                           (concat (gethash 'style omnivox-voice-table)
                                   " [{voice espeak:zlw/pl}]")))))
        (should (= attempts 1))
        (should (= (length warnings) 1))
        (should-not (process-get dtk-speaker-process
                                 'omnivox--styles-ready))))))

(ert-run-tests-batch-and-exit)

;;; omnivox-voices-tests.el ends here
