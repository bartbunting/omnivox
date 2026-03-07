;;; omnivox-voices.el --- Define Omnivox voice parameters -*- lexical-binding: t; -*-
;;
;; Description:  Module to set up Omnivox voices and personalities
;; Keywords: Voice, Personality, Omnivox
;;
;; Omnivox is a cross-platform Emacspeak speech server written in Rust.
;; It supports macOS (AVSpeechSynthesizer), Windows (WinRT), and
;; espeak-ng as a universal fallback.  Rate uses a 0-100 integer
;; scale (50 = normal speed), divided by 100 server-side.

;;;   Copyright:

;; Copyright (C) 1995 -- 2024, T. V. Raman
;; All Rights Reserved.
;;
;; This file is not part of GNU Emacs, but the same permissions apply.
;;
;; GNU Emacs is free software; you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation; either version 2, or (at your option)
;; any later version.
;;
;; GNU Emacs is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.
;;
;; You should have received a copy of the GNU General Public License
;; along with GNU Emacs; see the file COPYING.  If not, write to
;; the Free Software Foundation, 51 Franklin Street, Fifth Floor,
;; Boston, MA 02110-1301, USA.

;;; Commentary:
;; This module defines the various voices used in voice-lock mode
;; by the Omnivox TTS engine.
;;
;; Self-registering: this file hooks into emacspeak automatically.
;; No need to modify emacspeak files.  Just add to load-path and
;; require before emacspeak loads:
;;
;;   (add-to-list 'load-path "/path/to/omnivox/elisp")
;;   (require 'omnivox-voices)
;;
;; Omnivox inline codes:
;;   [{voice <name>}]  -- switch voice (e.g. en-US:Alex)
;;   [[pitch <float>]] -- set pitch multiplier (1.0 = normal)
;;
;; Voice querying:
;;   Voices are queried from the omnivox executable at runtime via
;;   --list-voices-alist.  The available voices vary by platform
;;   and installed voice packs.
;;
;; Customization:
;;   M-x customize-group RET omnivox RET
;;   All settings are in the `omnivox' customization group.
;;
;; Settings are sent to the running omnivox process via protocol commands.
;; No environment variables needed (omnivox uses CLI flags instead).

;;; Code:

;;  Required modules:

(eval-when-compile (require 'cl-lib))
(require 'emacspeak-preamble)
(cl-declaim  (optimize  (safety 0) (speed 3)))

;;;  Self-registration with emacspeak:
;;
;; These hooks run when emacspeak's modules load, injecting omnivox
;; support without modifying emacspeak source files.

(defun omnivox--voice-setup-advice (orig-fn)
  "Advice around `voice-setup' to handle omnivox.
If `dtk-program' matches \"omnivox\", configure omnivox and skip
the original dispatcher.  Otherwise delegate to ORIG-FN."
  (cl-declare (special dtk-program))
  (if (string-match "omnivox" dtk-program)
      (omnivox-configure-tts)
    (funcall orig-fn)))

(with-eval-after-load 'voice-setup
  (advice-add 'voice-setup :around #'omnivox--voice-setup-advice))

(with-eval-after-load 'dtk-speak
  (cl-declare (special tts-multi-engines))
  ;; Register omnivox as a multi-capable engine
  (cl-pushnew "omnivox" tts-multi-engines :test #'string=)
  ;; Advise notification initialization to set OMNIVOX_AUDIO_TARGET
  (advice-add 'dtk-notify-initialize :around #'omnivox--notify-advice))

(defun omnivox--notify-advice (orig-fn)
  "Advice around `dtk-notify-initialize' to set OMNIVOX_AUDIO_TARGET.
Binds the env var alongside the other engine-specific audio target
variables that emacspeak already handles."
  (cl-declare (special tts-notification-device))
  (let ((process-environment
         (cons (format "OMNIVOX_AUDIO_TARGET=%s"
                       (or (and (boundp 'tts-notification-device)
                                tts-notification-device)
                           ""))
               process-environment)))
    (funcall orig-fn)))

;;;  Customization group:

(defgroup omnivox nil
  "Omnivox cross-platform Emacspeak speech server."
  :group 'tts
  :prefix "omnivox-")

;;; omnivox:
;;;###autoload
(defun omnivox ()
  "Omnivox TTS."
  (interactive)
  (omnivox-configure-tts)
  (ems--fastload "voice-defs")
  (dtk-select-server "omnivox")
  (dtk-initialize))

;;;  Available voices (queried from server):

(defvar omnivox-available-voices nil
  "List of voices available from the omnivox server.
Each entry is (ID NAME LANGUAGE QUALITY).
Populated by `omnivox-refresh-voices'.")

(defun omnivox--find-executable ()
  "Find the omnivox executable.
Checks `emacspeak-servers-directory' first, then PATH."
  (cl-declare (special emacspeak-servers-directory))
  (let ((in-servers (expand-file-name "omnivox" emacspeak-servers-directory)))
    (if (file-executable-p in-servers)
        in-servers
      (executable-find "omnivox"))))

(defun omnivox-query-voices ()
  "Query available voices from the omnivox executable.
Returns a list of (ID NAME LANGUAGE QUALITY) entries."
  (let ((exe (omnivox--find-executable)))
    (when exe
      (condition-case nil
          (let* ((raw (shell-command-to-string
                       (format "%s --list-voices-alist 2>/dev/null" exe)))
                 ;; Find the start of the alist in case piper/ONNX printed
                 ;; anything to stdout before our output.
                 (start (string-search "(" raw)))
            (when start
              (car (read-from-string raw start))))
        (error nil)))))

;;;###autoload
(defun omnivox-refresh-voices ()
  "Refresh the list of available voices from the server."
  (interactive)
  (setq omnivox-available-voices (omnivox-query-voices))
  (when (called-interactively-p 'interactive)
    (message "Found %d voices" (length omnivox-available-voices))))

(defun omnivox-voice-ids ()
  "Return list of voice IDs (e.g. \"en-US:Alex\") from the server."
  (unless omnivox-available-voices
    (omnivox-refresh-voices))
  (mapcar #'car omnivox-available-voices))

;;;###autoload
(defun omnivox-list-voices ()
  "Display available voices in a help buffer."
  (interactive)
  (unless omnivox-available-voices
    (omnivox-refresh-voices))
  (with-help-window "*Omnivox Voices*"
    (with-current-buffer "*Omnivox Voices*"
      (insert (format "Omnivox: %d voices available\n\n"
                      (length omnivox-available-voices)))
      (let ((current-lang ""))
        (dolist (v omnivox-available-voices)
          (let ((id (nth 0 v))
                (name (nth 1 v))
                (lang (nth 2 v))
                (quality (nth 3 v)))
            (unless (string= lang current-lang)
              (unless (string-empty-p current-lang) (insert "\n"))
              (insert (format "%s:\n" lang))
              (setq current-lang lang))
            (insert (format "  %-30s  %s  %s\n" id name quality))))))))

;;;  Customizations:

(defcustom omnivox-speech-rate 60
  "Default speech rate for Omnivox.
Value is an integer on a 0-100 scale where 50 is normal speed,
lower is faster, higher is slower."
  :group 'omnivox
  :type 'integer
  :set #'(lambda (sym val)
           (set-default sym val)
           (when (and (string-match "omnivox\\'" dtk-program)
                      (boundp 'dtk-speaker-process)
                      (process-live-p dtk-speaker-process))
             (setq dtk-speech-rate val)
             (setq-default dtk-speech-rate val)
             (omnivox--send (format "tts_set_speech_rate %s" val)))))

(defcustom omnivox-voice-id ""
  "Default voice for Omnivox.
Value is a voice ID like \"en-US:Alex\".  Empty string means use server default.
Use `omnivox-select-voice' to interactively choose from available voices."
  :group 'omnivox
  :type 'string
  :set #'(lambda (sym val)
           (set-default sym val)
           (when (and (string-match "omnivox\\'" dtk-program)
                      (not (string-empty-p val))
                      (boundp 'dtk-speaker-process)
                      (process-live-p dtk-speaker-process))
             (omnivox--send (format "tts_set_voice %s" val)))))

(defcustom omnivox-pitch 1.0
  "Default pitch multiplier for Omnivox.
1.0 is normal pitch.  Range 0.5 (low) to 2.0 (high)."
  :group 'omnivox
  :type 'float
  :set #'(lambda (sym val)
           (set-default sym val)
           (when (and (string-match "omnivox\\'" dtk-program)
                      (boundp 'dtk-speaker-process)
                      (process-live-p dtk-speaker-process))
             (omnivox--send (format "tts_set_pitch_multiplier %s" val)))))

(defcustom omnivox-voice-volume 1.0
  "Default voice volume for Omnivox.
Float from 0.0 (silent) to 1.0 (full)."
  :group 'omnivox
  :type 'float
  :set #'(lambda (sym val)
           (set-default sym val)
           (when (and (string-match "omnivox\\'" dtk-program)
                      (boundp 'dtk-speaker-process)
                      (process-live-p dtk-speaker-process))
             (omnivox--send (format "tts_set_voice_volume %s" val)))))

(defcustom omnivox-tone-volume 0.1
  "Default tone volume for Omnivox.
Float from 0.0 (silent) to 1.0 (full)."
  :group 'omnivox
  :type 'float
  :set #'(lambda (sym val)
           (set-default sym val)
           (when (and (string-match "omnivox\\'" dtk-program)
                      (boundp 'dtk-speaker-process)
                      (process-live-p dtk-speaker-process))
             (omnivox--send (format "tts_set_tone_volume %s" val)))))

(defcustom omnivox-sound-volume 0.5
  "Default sound/audio-icon volume for Omnivox.
Float from 0.0 (silent) to 1.0 (full)."
  :group 'omnivox
  :type 'float
  :set #'(lambda (sym val)
           (set-default sym val)
           (when (and (string-match "omnivox\\'" dtk-program)
                      (boundp 'dtk-speaker-process)
                      (process-live-p dtk-speaker-process))
             (omnivox--send (format "tts_set_sound_volume %s" val)))))

;;;  Interactive commands (omnivox-specific, no dtk confusion):

(defun omnivox--send (command)
  "Send COMMAND string to the running omnivox process."
  (cl-declare (special dtk-speaker-process))
  (when (and (boundp 'dtk-speaker-process)
             (process-live-p dtk-speaker-process))
    (process-send-string
     dtk-speaker-process
     (concat command "\n"))))

;;;###autoload
(defun omnivox-select-voice ()
  "Interactively select a voice from the server's available voices."
  (interactive)
  (unless omnivox-available-voices
    (omnivox-refresh-voices))
  (unless omnivox-available-voices
    (error "No voices available from omnivox"))
  (let* ((candidates
          (mapcar (lambda (v)
                    (let ((id (nth 0 v))
                          (name (nth 1 v))
                          (quality (nth 3 v)))
                      (cons (format "%s [%s %s]" id name quality) id)))
                  omnivox-available-voices))
         (choice (completing-read "Voice: " candidates nil t))
         (voice-id (cdr (assoc choice candidates))))
    (when voice-id
      (omnivox--send (format "tts_set_voice %s" voice-id))
      (setq omnivox-voice-id voice-id)
      (message "Voice set to %s" voice-id))))

;;;###autoload
(defun omnivox-set-pitch (pitch)
  "Set Omnivox pitch multiplier to PITCH (0.5-2.0, 1.0 = normal)."
  (interactive "nPitch multiplier (0.5-2.0): ")
  (omnivox--send (format "tts_set_pitch_multiplier %s" pitch))
  (message "Pitch set to %s" pitch))

;;;###autoload
(defun omnivox-set-voice-volume (vol)
  "Set Omnivox voice volume to VOL (0.0-1.0)."
  (interactive "nVoice volume (0.0-1.0): ")
  (omnivox--send (format "tts_set_voice_volume %s" vol))
  (message "Voice volume set to %s" vol))

;;;###autoload
(defun omnivox-set-tone-volume (vol)
  "Set Omnivox tone volume to VOL (0.0-1.0)."
  (interactive "nTone volume (0.0-1.0): ")
  (omnivox--send (format "tts_set_tone_volume %s" vol))
  (message "Tone volume set to %s" vol))

;;;###autoload
(defun omnivox-set-sound-volume (vol)
  "Set Omnivox sound/audio-icon volume to VOL (0.0-1.0)."
  (interactive "nSound volume (0.0-1.0): ")
  (omnivox--send (format "tts_set_sound_volume %s" vol))
  (message "Sound volume set to %s" vol))

;;;###autoload
(defun omnivox-set-rate (rate)
  "Set Omnivox speech rate to RATE (0-100, 50 = normal speed)."
  (interactive "nSpeech rate (0-100): ")
  (cl-declare (special dtk-speech-rate))
  (set-default 'omnivox-speech-rate rate)
  (setq dtk-speech-rate rate)
  (setq-default dtk-speech-rate rate)
  (omnivox--send (format "tts_set_speech_rate %s" rate))
  (message "Rate set to %s" rate))

;;;###autoload
(defun omnivox-faster ()
  "Increase Omnivox speech rate by one step."
  (interactive)
  (cl-declare (special dtk-speech-rate dtk-speech-rate-step))
  (omnivox-set-rate (- dtk-speech-rate dtk-speech-rate-step)))

;;;###autoload
(defun omnivox-slower ()
  "Decrease Omnivox speech rate by one step."
  (interactive)
  (cl-declare (special dtk-speech-rate dtk-speech-rate-step))
  (omnivox-set-rate (+ dtk-speech-rate dtk-speech-rate-step)))

;;;###autoload
(defun omnivox-stop ()
  "Stop Omnivox speech immediately."
  (interactive)
  (dtk-stop))

;;;###autoload
(defun omnivox-speak-line ()
  "Speak the current line via Omnivox."
  (interactive)
  (dtk-speak-line))

;;;###autoload
(defun omnivox-status ()
  "Show current Omnivox settings."
  (interactive)
  (cl-declare (special dtk-speech-rate))
  (message "Voice: %s | Rate: %s | Pitch: %s | Volumes: voice=%s tone=%s sound=%s"
           (if (string-empty-p omnivox-voice-id) "default" omnivox-voice-id)
           dtk-speech-rate
           omnivox-pitch
           omnivox-voice-volume
           omnivox-tone-volume
           omnivox-sound-volume))

;;;   voice table

(defvar omnivox-voice-string "[[pitch 1]]"
  "Default Omnivox tag for the default voice.
Resets pitch to normal.  The actual voice is set via `omnivox-voice-id'.")

(defvar omnivox-voice-table (make-hash-table)
  "Association between symbols and strings to set Omnivox voices.
The string can set any voice parameter.")

(defun omnivox-define-voice (name command-string)
  "Define an Omnivox voice named NAME.
This voice will be set by sending the string
COMMAND-STRING to the TTS engine."
  (cl-declare (special omnivox-voice-table))
  (puthash name command-string omnivox-voice-table))

(defun omnivox-get-voice-command-internal (name)
  "Retrieve command string for voice NAME."
  (cl-declare (special omnivox-voice-table))
  (cond
   ((listp name)
    (mapconcat #'omnivox-get-voice-command name " "))
   (t (or (gethash name omnivox-voice-table)
          omnivox-voice-string))))

(defun omnivox-get-voice-command (name)
  "Retrieve command string for voice NAME."
  (omnivox-get-voice-command-internal name))

(defun omnivox-voice-defined-p (name)
  "Check if there is a voice named NAME defined."
  (cl-declare (special omnivox-voice-table))
  (gethash name omnivox-voice-table))

;;;  voice definitions

;; the predefined voices:
(omnivox-define-voice 'paul omnivox-voice-string)

;;;   Mapping css parameters to tts codes

;;;  voice family codes

(defun omnivox-get-family-code (name)
  "Get control code for voice family NAME."
  (omnivox-get-voice-command-internal name))

;;;   hash table for mapping families to their dimensions

(defvar omnivox-css-code-tables (make-hash-table)
  "Hash table holding vectors of Omnivox codes.
Keys are symbols of the form <FamilyName-Dimension>.
Values are vectors holding the control codes for the 10 settings.")

(defun omnivox-css-set-code-table (family dimension table)
  "Set up voice FAMILY.
Argument DIMENSION is the dimension being set,
and TABLE gives the values along that dimension."
  (cl-declare (special omnivox-css-code-tables))
  (let ((key (intern (format "%s-%s" family dimension))))
    (puthash key table omnivox-css-code-tables)))

(defun omnivox-css-get-code-table (family dimension)
  "Retrieve table of values for specified FAMILY and DIMENSION."
  (cl-declare (special omnivox-css-code-tables))
  (let ((key (intern (format "%s-%s" family dimension))))
    (gethash key omnivox-css-code-tables)))

;;;   average pitch

;; Omnivox uses a pitch multiplier (float) where 1.0 is normal.
;; We map CSS average-pitch settings 0-9 to pitch multiplier values:
;;   0 = 0.5 (very low), 5 = 1.0 (normal), 9 = 2.0 (very high)

;;;   paul average pitch

(let ((table (make-vector 10 "")))
  (mapc
   #'(lambda (setting)
       (aset table
             (cl-first setting)
             (format " [[pitch %s]] "
                     (cl-second setting))))
   '(
     (0 0.5)
     (1 0.6)
     (2 0.7)
     (3 0.8)
     (4 0.9)
     (5 1.0)
     (6 1.2)
     (7 1.4)
     (8 1.7)
     (9 2.0)))
  (omnivox-css-set-code-table 'paul 'average-pitch table))

(defun omnivox-get-average-pitch-code (value family)
  "Get AVERAGE-PITCH code for specified VALUE and FAMILY."
  (or family (setq family 'paul))
  (if value
      (aref (omnivox-css-get-code-table family 'average-pitch)
            value)
    ""))

;;;   pitch range

;; Omnivox does not currently support pitch-range control.
;; These are no-ops that produce empty strings.

(let ((table (make-vector 10 "")))
  (omnivox-css-set-code-table 'paul 'pitch-range table))

(defun omnivox-get-pitch-range-code (value family)
  "Get pitch-range code for specified VALUE and FAMILY."
  (or family (setq family 'paul))
  (if value
      (aref (omnivox-css-get-code-table family 'pitch-range)
            value)
    ""))

;;;   stress

;; Omnivox does not currently support stress control.
;; These are no-ops that produce empty strings.

(let ((table (make-vector 10 "")))
  (omnivox-css-set-code-table 'paul 'stress table))

(defun omnivox-get-stress-code (value family)
  "Get stress code for specified VALUE and FAMILY."
  (or family (setq family 'paul))
  (if value
      (aref (omnivox-css-get-code-table family 'stress)
            value)
    ""))

;;;   richness

;; Omnivox does not currently support richness control.
;; These are no-ops that produce empty strings.

(let ((table (make-vector 10 "")))
  (omnivox-css-set-code-table 'paul 'richness table))

(defun omnivox-get-richness-code (value family)
  "Get richness code for specified VALUE and FAMILY."
  (or family (setq family 'paul))
  (if value
      (aref (omnivox-css-get-code-table family 'richness)
            value)
    ""))

;;;   omnivox-define-voice-from-acss

(defun omnivox-define-voice-from-acss (name style)
  "Define NAME to be an Omnivox voice as specified by settings in STYLE."
  (let* ((family (acss-family style))
         (command
          (concat
           (omnivox-get-family-code family)
           (omnivox-get-average-pitch-code (acss-average-pitch style) family)
           (omnivox-get-pitch-range-code (acss-pitch-range style) family)
           (omnivox-get-stress-code (acss-stress style) family)
           (omnivox-get-richness-code (acss-richness style) family))))
    (omnivox-define-voice name command)))

;;;  Configure TTS:
;;;###autoload
(defun omnivox-configure-tts ()
  "Configure TTS to use Omnivox.
Sends defcustom settings to the already-running omnivox process
via protocol commands."
  (cl-declare (special tts-default-speech-rate
                       tts-default-voice))
  (setq tts-default-voice 'paul)
  (fset 'tts-voice-defined-p 'omnivox-voice-defined-p)
  (fset 'tts-get-voice-command 'omnivox-get-voice-command)
  (fset 'tts-define-voice-from-acss 'omnivox-define-voice-from-acss)
  ;; Apply rate — dtk-speech-rate is a buffer-local integer used by
  ;; dtk-interp-sync via tts_sync_state on every utterance.  Must set
  ;; BOTH the current-buffer value and the global default, otherwise
  ;; buffers created before configure-tts keep the old default (100).
  ;; Omnivox uses 0-100 integer scale (divided by 100 server-side).
  (cl-declare (special dtk-speech-rate dtk-speech-rate-base dtk-speech-rate-step))
  (setq tts-default-speech-rate omnivox-speech-rate)
  (set-default 'tts-default-speech-rate omnivox-speech-rate)
  (setq dtk-speech-rate omnivox-speech-rate)
  (setq-default dtk-speech-rate omnivox-speech-rate)
  (setq dtk-speech-rate-base 20)
  (setq dtk-speech-rate-step 5)
  (dtk-unicode-update-untouched-charsets
   '(ascii latin-iso8859-1 latin-iso8859-15 latin-iso8859-9
           eight-bit-graphic))
  (setq emacspeak-play-program nil)
  ;; Send settings to the running omnivox process via protocol commands.
  ;; The process was already started by dtk-make-process before voice-setup
  ;; called us, so protocol commands are the reliable way to configure it.
  (omnivox--send (format "tts_set_speech_rate %s" omnivox-speech-rate))
  (omnivox--send (format "tts_set_pitch_multiplier %s" omnivox-pitch))
  (omnivox--send (format "tts_set_voice_volume %s" omnivox-voice-volume))
  (omnivox--send (format "tts_set_tone_volume %s" omnivox-tone-volume))
  (omnivox--send (format "tts_set_sound_volume %s" omnivox-sound-volume))
  (unless (string-empty-p omnivox-voice-id)
    (omnivox--send (format "tts_set_voice %s" omnivox-voice-id)))
  ;; Query available voices
  (omnivox-refresh-voices))

(provide 'omnivox-voices)
