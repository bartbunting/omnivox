;;; Temporary native profile registration observations; no control credentials.
(defvar native-registry-trace nil)
(defun native-registry-note (event)
  (push (cons (float-time) event) native-registry-trace)
  (when (> (length native-registry-trace) 100) (setcdr (nthcdr 99 native-registry-trace) nil)))
(with-eval-after-load 'omnivox-voices
  (advice-add 'omnivox--unchanged-native-registry-generation :around
    (lambda (original process content)
      (let* ((old (process-get process omnivox--choice-registration-property))
             (before (plist-get old :content)) (reuse (funcall original process content)))
        (native-registry-note
          (list :event 'reuse :lane (process-name process)
                :ack (plist-get old :registry-generation)
                :sent (process-get process 'omnivox--last-sent-registry-generation)
                :reuse reuse
                :same-wire (and before (equal (json-serialize (omnivox--registration-request 1 content))
                                             (json-serialize (omnivox--registration-request 1 before))))
                :same-private (mapcar (lambda (key) (cons key (equal (plist-get content key) (plist-get before key))))
                  '(:choice-process-generation :choice-tuning-unapplied :choice-provenance))))
        reuse)))
  (advice-add 'omnivox--send-control-request :before
    (lambda (process request &rest _)
      (when (member (plist-get request :type) '("register_logical_voices_v3" "set_routing_policy"))
        (native-registry-note (list :event 'send :lane (process-name process) :type (plist-get request :type)
                                   :generation (plist-get request :registry_generation)
                                   :same-ack-wire
                                   (let ((old (plist-get (process-get process omnivox--choice-registration-property) :content))
                                         (copy (copy-tree request)))
                                     (when old
                                       (setf (plist-get copy :registry_generation) 1)
                                       (equal (json-serialize copy)
                                              (json-serialize (omnivox--registration-request 1 old))))))))))
  (advice-add 'omnivox--accept-registration-response :before
    (lambda (process response generation _content)
      (native-registry-note (list :event 'ack :lane (process-name process) :generation generation
                                 :request (plist-get response :request_id) :type (plist-get response :type))))))
(with-eval-after-load 'omnivox-voices
  (advice-add 'omnivox-apply-voice-configuration :before
    (lambda (&rest _)
      (native-registry-note (list :event 'apply :callers
                                 (mapcar (lambda (frame) (nth 1 frame)) (backtrace-frames)))))))
(load (expand-file-name "bin/evox-windows-init.el" (getenv "USERPROFILE")) nil t)
