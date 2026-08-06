//! Protocol server: synthesis worker thread, reader loop, command dispatch.

use anyhow::Result;
use omnivox_audio::{AudioControl, AudioFileLoader, StreamType, ToneGenerator};
use omnivox_core::{
    parse_command, state::{ChannelMode, PunctuationLevel}, Command, CommandId, QueueItem, TtsState,
};
use omnivox_tts::{TtsEngine, TtsSettings};
use std::io::{self, BufRead};
use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use tracing::{debug, error, info, warn};

use crate::pipeline::{
    build_sound_pipeline, build_tone_pipeline, process_batch, synthesize_chunk, SynthCtx,
};
use crate::text::{chunk_text, normalize_rate, parse_resource_path, preprocess_text};

// ---------------------------------------------------------------------------
// Synthesis request types
// ---------------------------------------------------------------------------

/// Messages sent from the reader thread to the synthesis worker.
///
/// Each request carries a `gen` (generation) stamp. The worker compares it
/// against the shared `gen_counter` before and after each synthesis call; if
/// the counter has advanced (because the reader processed a `s` / `tts_say`
/// interrupt), the request is abandoned and no audio is queued.
pub enum SynthRequest {
    /// Synthesize and play a batch of queued items (from `q`/`c`/`t`/`sh`/`a` + `d`).
    Batch { items: Vec<QueueItem>, state: TtsState, gen: u64 },
    /// Synthesize and play a single string immediately (`tts_say`).
    Immediate { text: String, state: TtsState, gen: u64 },
    /// Synthesize and play a single letter (`l`).
    Letter { text: String, state: TtsState, gen: u64 },
    /// Play a sound file immediately on the sound stream (`p`).
    PlaySound { path: std::path::PathBuf, state: TtsState, gen: u64 },
}

// ---------------------------------------------------------------------------
// Synthesis worker
// ---------------------------------------------------------------------------

/// Worker thread: receive `SynthRequest`s and synthesize them one at a time.
pub fn synthesis_worker(
    rx: mpsc::Receiver<SynthRequest>,
    gen_counter: Arc<AtomicU64>,
    engine: Arc<dyn TtsEngine>,
    control: Arc<AudioControl>,
    loader: AudioFileLoader,
) {
    for request in rx {
        match request {
            SynthRequest::Batch { items, state, gen } => {
                let ctx = SynthCtx { gen, gen_counter: &gen_counter, engine: &*engine, control: &control };
                process_batch(items, state, &ctx, &loader);
            }

            SynthRequest::Immediate { text, state, gen } => {
                let ctx = SynthCtx { gen, gen_counter: &gen_counter, engine: &*engine, control: &control };
                if ctx.is_stale() { continue; }
                let settings = TtsSettings {
                    voice: state.current_voice.clone(),
                    rate: state.speech_rate,
                    pitch: state.pitch_multiplier,
                    volume: 1.0,
                };
                let processed = preprocess_text(&text, &state);
                let chunks = chunk_text(&processed, 15);
                let count = chunks.len();
                for (i, chunk) in chunks.into_iter().enumerate() {
                    if !synthesize_chunk(&chunk, &settings, &state, i == count - 1, &ctx) {
                        break;
                    }
                }
            }

            SynthRequest::Letter { text, state, gen } => {
                let ctx = SynthCtx { gen, gen_counter: &gen_counter, engine: &*engine, control: &control };
                if ctx.is_stale() { continue; }

                let mut letter_state = state.clone();
                letter_state.speech_rate = state.character_rate();

                let is_upper = text.chars().next().is_some_and(|c| c.is_uppercase());
                if is_upper {
                    if state.allcaps_beep {
                        let mut tone_buf = ToneGenerator::generate(440.0, 10, state.tone_volume);
                        let pipeline = build_tone_pipeline(&state);
                        let _ = pipeline.process(&mut tone_buf);
                        let _ = ctx.control.queue(StreamType::Tone, &tone_buf);
                    } else {
                        letter_state.pitch_multiplier = 1.5;
                    }
                }

                let settings = TtsSettings {
                    voice: letter_state.current_voice.clone(),
                    rate: letter_state.speech_rate,
                    pitch: letter_state.pitch_multiplier,
                    volume: 1.0,
                };
                synthesize_chunk(&text.to_ascii_lowercase(), &settings, &letter_state, true, &ctx);
            }

            SynthRequest::PlaySound { path, state, gen } => {
                let ctx = SynthCtx { gen, gen_counter: &gen_counter, engine: &*engine, control: &control };
                if ctx.is_stale() { continue; }
                match loader.load(&path) {
                    Ok(mut buf) => {
                        let pipeline = build_sound_pipeline(&state);
                        if let Err(e) = pipeline.process(&mut buf) {
                            warn!("Sound pipeline error: {}", e);
                        }
                        if let Err(e) = ctx.control.queue(StreamType::Sound, &buf) {
                            warn!("Sound queue error: {}", e);
                        }
                    }
                    Err(e) => warn!("Failed to load sound {}: {}", path.display(), e),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Reader loop
// ---------------------------------------------------------------------------

/// Increment the generation counter, stop audio, and optionally stop the TTS engine.
///
/// `stop_engine` should be `true` only for hard stops (`s` command).  For
/// `tts_say` and `letter`, pass `false` — the generation counter already causes
/// the worker to discard stale results, and calling `engine.stop()` cross-thread
/// while AVSpeechSynthesizer is running on its GCD queue corrupts the synthesizer.
pub fn interrupt(
    current_gen: &mut u64,
    gen_counter: &AtomicU64,
    control: &AudioControl,
    engine: &dyn TtsEngine,
    stop_speech_only: bool,
    stop_engine: bool,
) {
    *current_gen += 1;
    gen_counter.store(*current_gen, Ordering::Release);
    if stop_speech_only {
        control.stop(StreamType::Speech);
    } else {
        control.stop_all();
    }
    if stop_engine {
        engine.stop();
    }
}

/// Reader loop: process stdin commands and drive the synthesis worker.
///
/// Does not own `AudioStreams` — the caller keeps it alive so the `OutputStream`
/// drop guard outlives playback.
pub fn run_server(
    engine: Arc<dyn TtsEngine>,
    mut state: TtsState,
    tx: mpsc::Sender<SynthRequest>,
    control: Arc<AudioControl>,
    gen_counter: Arc<AtomicU64>,
    worker_handle: std::thread::JoinHandle<()>,
) -> Result<()> {
    let mut pending: Vec<QueueItem> = Vec::new();
    let mut current_gen: u64 = 0;

    info!("Ready to accept commands from stdin");

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        debug!("Received: {}", line);

        let command = match parse_command(line) {
            Ok(c) => c,
            Err(e) => {
                error!("Parse error '{}': {}", line, e);
                continue;
            }
        };

        handle_command(command, &mut state, &mut pending, &mut current_gen, &gen_counter, &engine, &control, &tx);
    }

    info!("Stdin closed; waiting for synthesis worker to finish");
    drop(tx);
    let _ = worker_handle.join();

    info!("Draining audio output");
    control.drain();

    info!("Shutting down");
    Ok(())
}

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn handle_command(
    command: Command,
    state: &mut TtsState,
    pending: &mut Vec<QueueItem>,
    current_gen: &mut u64,
    gen_counter: &Arc<AtomicU64>,
    engine: &Arc<dyn TtsEngine>,
    control: &Arc<AudioControl>,
    tx: &mpsc::Sender<SynthRequest>,
) {
    match command.id {
        // --- Queue accumulation (no synthesis yet) ---

        CommandId::Queue => {
            if let Some(text) = command.args {
                debug!("Queue speech: {}", text);
                pending.push(QueueItem::Speech(text));
            }
        }

        CommandId::Code => {
            if let Some(codes) = command.args {
                debug!("Queue codes: {}", codes);
                pending.push(QueueItem::Code(codes));
            }
        }

        CommandId::Tone => {
            if let Some(args) = command.args {
                let parts: Vec<&str> = args.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let (Ok(freq), Ok(dur)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                        debug!("Queue tone: {}Hz {}ms", freq, dur);
                        pending.push(QueueItem::Tone { frequency: freq, duration: dur });
                    }
                }
            }
        }

        CommandId::Silence => {
            if let Some(dur_str) = command.args {
                if let Ok(dur) = dur_str.parse::<u32>() {
                    debug!("Queue silence: {}ms", dur);
                    pending.push(QueueItem::Silence { duration: dur });
                }
            }
        }

        CommandId::AudioIcon => {
            if let Some(path) = command.args {
                match parse_resource_path(&path) {
                    Ok(path) => {
                        debug!("Queue audio icon: {}", path.display());
                        pending.push(QueueItem::AudioIcon { path });
                    }
                    Err(error) => warn!("Invalid audio icon path: {}", error),
                }
            }
        }

        // --- Dispatch: send accumulated items to worker ---

        CommandId::Dispatch => {
            if !pending.is_empty() {
                debug!("Dispatch {} items (gen={})", pending.len(), current_gen);
                let items = mem::take(pending);
                let _ = tx.send(SynthRequest::Batch { items, state: state.clone(), gen: *current_gen });
            }
        }

        // --- Interrupting commands ---

        CommandId::Stop => {
            debug!("Stop");
            interrupt(current_gen, gen_counter, control, engine.as_ref(), false, true);
            pending.clear();
        }

        CommandId::TtsSay => {
            if let Some(text) = command.args {
                debug!("tts_say: {}", text);
                interrupt(current_gen, gen_counter, control, engine.as_ref(), true, false);
                let _ = tx.send(SynthRequest::Immediate { text, state: state.clone(), gen: *current_gen });
            }
        }

        CommandId::Letter => {
            if let Some(letter) = command.args {
                debug!("Letter: {}", letter);
                interrupt(current_gen, gen_counter, control, engine.as_ref(), true, false);
                let _ = tx.send(SynthRequest::Letter { text: letter, state: state.clone(), gen: *current_gen });
            }
        }

        CommandId::PlaySound => {
            if let Some(path) = command.args {
                match parse_resource_path(&path) {
                    Ok(path) => {
                        debug!("Play sound: {}", path.display());
                        let _ = tx.send(SynthRequest::PlaySound {
                            path,
                            state: state.clone(),
                            gen: *current_gen,
                        });
                    }
                    Err(error) => warn!("Invalid sound path: {}", error),
                }
            }
        }

        CommandId::Version => {
            let version_text = format!(
                "Omnivox version {}",
                crate::VERSION.replace('.', " dot ")
            );
            let _ = tx.send(SynthRequest::Immediate { text: version_text, state: state.clone(), gen: *current_gen });
        }

        // --- State management ---

        CommandId::TtsSetSpeechRate => {
            if let Some(rate) = command.args {
                if let Ok(r) = rate.parse::<f32>() {
                    state.speech_rate = normalize_rate(r);
                    debug!("Speech rate: {}", state.speech_rate);
                }
            }
        }

        CommandId::TtsSetVoice => {
            if let Some(voice) = command.args {
                debug!("Voice: {}", voice);
                state.current_voice = voice;
            }
        }

        CommandId::TtsSetPitchMultiplier => {
            if let Some(pitch) = command.args {
                if let Ok(p) = pitch.parse::<f32>() {
                    state.pitch_multiplier = p;
                    debug!("Pitch: {}", p);
                }
            }
        }

        CommandId::TtsSetVoiceVolume => {
            if let Some(vol) = command.args {
                if let Ok(v) = vol.parse::<f32>() {
                    state.voice_volume = v;
                }
            }
        }

        CommandId::TtsSetToneVolume => {
            if let Some(vol) = command.args {
                if let Ok(v) = vol.parse::<f32>() {
                    state.tone_volume = v;
                }
            }
        }

        CommandId::TtsSetSoundVolume => {
            if let Some(vol) = command.args {
                if let Ok(v) = vol.parse::<f32>() {
                    state.sound_volume = v;
                }
            }
        }

        CommandId::TtsSetCharacterScale => {
            if let Some(scale) = command.args {
                if let Ok(s) = scale.parse::<f32>() {
                    state.character_scale = s;
                }
            }
        }

        CommandId::TtsSplitCaps => {
            if let Some(flag) = command.args {
                state.split_caps = flag == "1";
            }
        }

        CommandId::TtsAllCapsBeep => {
            if let Some(flag) = command.args {
                state.allcaps_beep = flag == "1";
            }
        }

        CommandId::TtsSetPunctuations => {
            if let Some(level) = command.args {
                if let Some(punct) = PunctuationLevel::parse(&level) {
                    state.punctuation_level = punct;
                }
            }
        }

        CommandId::TtsSyncState => {
            if let Some(args) = command.args {
                let parts: Vec<&str> = args.split_whitespace().collect();
                if parts.len() >= 4 {
                    if let Some(punct) = PunctuationLevel::parse(parts[0]) {
                        state.punctuation_level = punct;
                    }
                    state.split_caps = parts[1] == "1";
                    state.allcaps_beep = parts[2] == "1";
                    if let Ok(r) = parts[3].parse::<f32>() {
                        state.speech_rate = normalize_rate(r);
                    }
                }
            }
        }

        CommandId::TtsSetSpeechChannel => {
            if let Some(target) = command.args {
                if let Some(mode) = ChannelMode::parse(&target) {
                    state.speech_routing.channel_mode = mode;
                    debug!("Speech channel: {}", target);
                } else {
                    warn!("Invalid tts_set_speech_channel value: {}", target);
                }
            }
        }

        CommandId::TtsSetNotificationChannel => {
            if let Some(target) = command.args {
                if let Some(mode) = ChannelMode::parse(&target) {
                    state.notification_routing.channel_mode = mode;
                    debug!("Notification channel: {}", target);
                } else {
                    warn!("Invalid tts_set_notification_channel value: {}", target);
                }
            }
        }

        CommandId::TtsReset => {
            debug!("Reset");
            interrupt(current_gen, gen_counter, control, engine.as_ref(), false, true);
            state.reset();
            pending.clear();
        }

        CommandId::TtsExit => {
            info!("Exit command received");
            std::process::exit(0);
        }

        CommandId::SetLang | CommandId::SetNextLang | CommandId::SetPreviousLang | CommandId::SetPreferredLang => {
            debug!("Language switching not yet implemented: {:?} {:?}", command.id, command.args);
        }
    }
}
