//! Omnivox CLI - Emacspeak Speech Server
//!
//! Cross-platform text-to-speech server implementing the Emacspeak protocol.
//! Uses a buffer-based audio pipeline: TTS/tone/file -> pipeline -> output.
//!
//! # Threading Model
//!
//! The server uses two threads to ensure Emacs never blocks:
//!
//! - **Reader thread** (main): reads stdin and parses commands in a tight loop.
//!   Never blocks on synthesis. Stop/reset commands take effect immediately.
//!
//! - **Synthesis worker** (spawned): receives `SynthRequest`s via an unbounded
//!   channel, synthesizes each chunk, checks the generation counter between
//!   chunks, and queues audio to rodio. Stale requests (from before the last
//!   `s` or `tts_say`) are detected via the generation counter and skipped.
//!
//! Audio is played on three concurrent streams (speech, tones, sounds).
//! Items within each stream serialize; different streams overlap.

use anyhow::Result;
use omnivox_audio::{
    AudioBuffer, AudioControl, AudioFileLoader, AudioPipeline, AudioStreams, ChannelRouter,
    SilenceTrimmer, StreamType, ToneGenerator, VolumeAdjust,
};
use omnivox_core::{
    parse_command,
    state::{ChannelMode, PunctuationLevel},
    Command, CommandId, QueueItem, TtsState,
};
use omnivox_tts::espeak::EspeakTtsEngine;
#[cfg(target_os = "macos")]
use omnivox_tts::macos::MacOsTtsEngine;
#[cfg(target_os = "windows")]
use omnivox_tts::windows::WindowsTtsEngine;
use omnivox_tts::{TtsEngine, TtsSettings};
use once_cell::sync::Lazy;
use std::io::{self, BufRead, Write as IoWrite};
use std::mem;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Maximum queued items per audio stream before overflow drops old items.
const SPEECH_MAX_DEPTH: usize = 100;
const TONE_MAX_DEPTH: usize = 10;
const SOUND_MAX_DEPTH: usize = 10;

// ---------------------------------------------------------------------------
// Lazy-compiled regexes (compiled once, reused on every call)
// ---------------------------------------------------------------------------

static VOICE_RE: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r"\[\{voice\s+([^\}]+)\}\]").expect("invalid voice regex"));

static PITCH_RE: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r"\[\[pitch\s+([^\]]+)\]\]").expect("invalid pitch regex"));

// ---------------------------------------------------------------------------
// Synthesis requests
// ---------------------------------------------------------------------------

/// Messages sent from the reader thread to the synthesis worker.
///
/// Each request carries a `gen` (generation) stamp. The worker compares it
/// against the shared `gen_counter` before and after each synthesis call; if
/// the counter has advanced (because the reader processed a `s` / `tts_say`
/// interrupt), the request is abandoned and no audio is queued.
enum SynthRequest {
    /// Synthesize and play a batch of queued items (from `q`/`c`/`t`/`sh`/`a` + `d`).
    Batch {
        items: Vec<QueueItem>,
        state: TtsState,
        gen: u64,
    },
    /// Synthesize and play a single string immediately (`tts_say`).
    Immediate {
        text: String,
        state: TtsState,
        gen: u64,
    },
    /// Synthesize and play a single letter (`l`).
    Letter {
        text: String,
        state: TtsState,
        gen: u64,
    },
    /// Play a sound file immediately on the sound stream (`p`).
    PlaySound {
        path: PathBuf,
        state: TtsState,
        gen: u64,
    },
}

// ---------------------------------------------------------------------------
// Audio helpers
// ---------------------------------------------------------------------------

/// Convert a TTS AudioBuffer (omnivox_tts) to the pipeline AudioBuffer (omnivox_audio).
fn tts_buffer_to_audio_buffer(tts_buf: omnivox_tts::AudioBuffer) -> AudioBuffer {
    if tts_buf.is_empty() {
        return AudioBuffer::empty();
    }
    AudioBuffer::new(tts_buf.samples)
}

/// Split text into chunks of `max_words` words.
///
/// Keeps individual utterances small so the TTS engine produces single-buffer
/// output, enabling aggressive silence trimming and fast cancellation between
/// chunks.
fn chunk_text(text: &str, max_words: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();

    if words.len() <= max_words {
        return vec![text.to_string()];
    }

    words
        .chunks(max_words)
        .map(|chunk| chunk.join(" "))
        .collect()
}

/// Replace punctuation characters with spoken names based on punctuation level.
fn apply_punctuation(text: &str, level: PunctuationLevel) -> String {
    let mut result = String::with_capacity(text.len());

    for ch in text.chars() {
        let replacement = match level {
            PunctuationLevel::None => match ch {
                '$' => Some(" dollar "),
                '%' => Some(" percent "),
                _ => None,
            },
            PunctuationLevel::Some => match ch {
                '$' => Some(" dollar "),
                '%' => Some(" percent "),
                '#' => Some(" pound "),
                '-' => Some(" dash "),
                '"' => Some(" quote "),
                '(' => Some(" left paren "),
                ')' => Some(" right paren "),
                '*' => Some(" star "),
                ';' => Some(" semicolon "),
                ':' => Some(" colon "),
                '<' => Some(" less than "),
                '>' => Some(" greater than "),
                '\\' => Some(" backslash "),
                '/' => Some(" slash "),
                '+' => Some(" plus "),
                '=' => Some(" equals "),
                '~' => Some(" tilde "),
                '`' => Some(" backquote "),
                '!' => Some(" bang "),
                '^' => Some(" caret "),
                _ => None,
            },
            PunctuationLevel::All => match ch {
                '$' => Some(" dollar "),
                '%' => Some(" percent "),
                '#' => Some(" pound "),
                '-' => Some(" dash "),
                '"' => Some(" quote "),
                '(' => Some(" left paren "),
                ')' => Some(" right paren "),
                '*' => Some(" star "),
                ';' => Some(" semicolon "),
                ':' => Some(" colon "),
                '<' => Some(" less than "),
                '>' => Some(" greater than "),
                '\\' => Some(" backslash "),
                '/' => Some(" slash "),
                '+' => Some(" plus "),
                '=' => Some(" equals "),
                '~' => Some(" tilde "),
                '`' => Some(" backquote "),
                '!' => Some(" bang "),
                '^' => Some(" caret "),
                '@' => Some(" at "),
                '_' => Some(" underline "),
                '\'' => Some(" apostrophe "),
                '.' => Some(" dot "),
                ',' => Some(" comma "),
                '&' => Some(" ampersand "),
                '|' => Some(" pipe "),
                '[' => Some(" left bracket "),
                ']' => Some(" right bracket "),
                '{' => Some(" left brace "),
                '}' => Some(" right brace "),
                '?' => Some(" question "),
                _ => None,
            },
        };

        match replacement {
            Some(spoken) => result.push_str(spoken),
            None => result.push(ch),
        }
    }

    result
}

fn home_dir() -> Option<std::ffi::OsString> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return PathBuf::from(home).join(rest);
        }
    } else if path == "~" {
        if let Some(home) = home_dir() {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(path)
}

fn preprocess_text(text: &str, state: &TtsState) -> String {
    let mut processed = apply_punctuation(text, state.punctuation_level);
    if state.split_caps {
        processed = insert_space_before_uppercase(&processed);
    }
    processed
}

fn apply_audio_target_env(state: &mut TtsState) {
    if let Ok(target) = std::env::var("OMNIVOX_AUDIO_TARGET") {
        if let Some(channel_mode) = ChannelMode::parse(&target) {
            info!("Setting audio target from env: {}", target);
            state.speech_routing.channel_mode = channel_mode;
            state.notification_routing.channel_mode = channel_mode;
            state.tone_routing.channel_mode = channel_mode;
            state.sound_routing.channel_mode = channel_mode;
        } else {
            warn!("Invalid OMNIVOX_AUDIO_TARGET value: {}", target);
        }
    }
}

fn normalize_rate(rate: f32) -> f32 {
    let r = if rate > 1.0 { rate / 100.0 } else { rate };
    r.clamp(0.0, 1.0)
}

fn rate_scaled_padding(rate: f32) -> f32 {
    let rate = rate.clamp(0.0, 1.0);
    0.002 + 0.013 * (1.0 - rate)
}

fn build_speech_pipeline(state: &TtsState, is_first: bool, is_last: bool) -> AudioPipeline {
    let padding = rate_scaled_padding(state.speech_rate);
    let leading = if is_first { padding } else { 0.0 };
    let trailing = if is_last { padding } else { 0.0 };

    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(SilenceTrimmer::with_asymmetric_padding(
        0.01, leading, trailing,
    )));
    pipeline.push(Box::new(VolumeAdjust::new(state.voice_volume)));
    pipeline.push(Box::new(ChannelRouter::new(
        state.speech_routing.channel_mode,
    )));
    pipeline
}

fn build_tone_pipeline(state: &TtsState) -> AudioPipeline {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(VolumeAdjust::new(state.tone_volume)));
    pipeline.push(Box::new(ChannelRouter::new(
        state.tone_routing.channel_mode,
    )));
    pipeline
}

fn build_sound_pipeline(state: &TtsState) -> AudioPipeline {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(VolumeAdjust::new(state.sound_volume)));
    pipeline.push(Box::new(ChannelRouter::new(
        state.sound_routing.channel_mode,
    )));
    pipeline
}

// ---------------------------------------------------------------------------
// Engine creation
// ---------------------------------------------------------------------------

fn create_engine(engine_name: &str) -> Result<Arc<dyn TtsEngine>> {
    let forced = if engine_name.is_empty() {
        std::env::var("OMNIVOX_ENGINE").unwrap_or_default()
    } else {
        engine_name.to_string()
    };

    if forced != "espeak" {
        #[cfg(target_os = "macos")]
        {
            match MacOsTtsEngine::new() {
                Ok(engine) => {
                    info!("Using macOS AVSpeechSynthesizer engine");
                    return Ok(Arc::new(engine));
                }
                Err(e) => {
                    warn!("macOS TTS not available: {}, falling back to espeak-ng", e);
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            match WindowsTtsEngine::new() {
                Ok(engine) => {
                    info!("Using Windows WinRT engine");
                    return Ok(Arc::new(engine));
                }
                Err(e) => {
                    warn!("Windows WinRT not available: {}, falling back to espeak-ng", e);
                }
            }
        }
    }

    match EspeakTtsEngine::new() {
        Ok(engine) => {
            info!("Using espeak-ng engine");
            Ok(Arc::new(engine))
        }
        Err(e) => {
            anyhow::bail!("No TTS engine available: {}", e);
        }
    }
}

fn native_engine_name() -> &'static str {
    #[cfg(target_os = "macos")]
    { "macos (AVSpeechSynthesizer)" }
    #[cfg(target_os = "windows")]
    { "winrt (Windows SpeechSynthesizer)" }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { "none (espeak-ng is the only backend)" }
}

// ---------------------------------------------------------------------------
// Non-server commands (--help, --check, etc.)
// ---------------------------------------------------------------------------

fn print_help() {
    let native = native_engine_name();
    println!("Omnivox v{} - Cross-platform Emacspeak speech server", VERSION);
    println!();
    println!("USAGE:");
    println!("    omnivox [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    --help           Show this help message");
    println!("    --version        Show version number");
    println!("    --check          Run diagnostic self-test");
    println!("    --list-voices    List available TTS voices");
    println!("    --list-voices-alist  List voices as Emacs-readable alist");
    println!("    --engine NAME    Select TTS engine: native, espeak");
    println!("    --voice ID       Set default voice (e.g. en-US:Alex)");
    println!("    --rate FLOAT     Speech rate 0.0-1.0 (0.5 = normal)");
    println!("    --pitch FLOAT    Pitch multiplier 0.5-2.0 (1.0 = normal)");
    println!("    --voice-volume F Voice volume 0.0-1.0");
    println!("    --tone-volume F  Tone volume 0.0-1.0");
    println!("    --sound-volume F Sound/icon volume 0.0-1.0");
    println!("    --audio-target T Channel routing (left, right, both)");
    println!("    --dump-wav VOICE OUTPUT [TEXT]");
    println!("                     Save TTS output to WAV files for analysis");
    println!("    --play-wav FILE  Play a WAV file through the rodio audio path");
    println!();
    println!("ENGINES:");
    println!("    native    Platform-native TTS: {}", native);
    println!("    espeak    espeak-ng (cross-platform, always available)");
    println!();
    println!("Without options, starts the Emacspeak protocol server on stdin.");
    println!();
    println!("ENVIRONMENT (for Emacspeak integration only):");
    println!("    OMNIVOX_ENGINE         Same as --engine");
    println!("    OMNIVOX_AUDIO_TARGET   Same as --audio-target (set by Emacspeak notification mode)");
    println!();
    println!("EMACSPEAK SETUP:");
    println!("    (setq dtk-program \"omnivox\")");
    println!("    Ensure omnivox is in your PATH or in emacspeak/servers/");
}

fn print_version() {
    println!("omnivox {}", VERSION);
}

fn cmd_list_voices(engine: &dyn TtsEngine) {
    let voices = engine.available_voices();
    println!("Found {} voices:\n", voices.len());

    let mut by_lang: std::collections::BTreeMap<String, Vec<_>> = std::collections::BTreeMap::new();
    for voice in voices {
        by_lang.entry(voice.language.clone()).or_default().push(voice);
    }
    for (lang, voices) in by_lang {
        println!("{} ({} voices):", lang, voices.len());
        for voice in voices {
            println!("  {:?} - {} [{}]", voice.quality, voice.name, voice.identifier);
        }
        println!();
    }
}

fn cmd_list_voices_alist(engine: &dyn TtsEngine) {
    let voices = engine.available_voices();
    print!("(");
    for (i, voice) in voices.iter().enumerate() {
        let quality = format!("{:?}", voice.quality);
        let escaped_name = voice.name.replace('\\', "\\\\").replace('"', "\\\"");
        print!(
            "(\"{lang}:{name}\" \"{display}\" \"{lang}\" \"{quality}\")",
            lang = voice.language,
            name = escaped_name,
            display = escaped_name,
            quality = quality
        );
        if i < voices.len() - 1 {
            print!("\n ");
        }
    }
    println!(")");
}

fn cmd_check(engine_name: &str) {
    println!("Omnivox v{} diagnostic check", VERSION);
    println!("=============================\n");

    println!("[platform]");
    println!("  OS: {}", std::env::consts::OS);
    println!("  Arch: {}", std::env::consts::ARCH);
    println!("  Native engine: {}", native_engine_name());
    println!();

    println!("[home]");
    match home_dir() {
        Some(h) => println!("  Home: {}", std::path::Path::new(&h).display()),
        None => println!("  WARNING: Could not determine home directory (HOME/USERPROFILE not set)"),
    }
    println!();

    println!("[engine]");
    let engine: Arc<dyn TtsEngine> = match create_engine(engine_name) {
        Ok(e) => {
            println!("  Status: OK");
            e
        }
        Err(e) => {
            println!("  Status: FAILED - {}", e);
            println!("\nDiagnostic check failed: no TTS engine available.");
            std::process::exit(1);
        }
    };

    let voices = engine.available_voices();
    println!("  Voices: {}", voices.len());
    if voices.is_empty() {
        println!("  WARNING: No voices found");
    } else {
        for v in &voices {
            println!("    - {} ({}) [{:?}]", v.name, v.language, v.quality);
        }
    }
    println!();

    println!("[synthesis]");
    let settings = TtsSettings::default();
    match engine.synthesize("test", &settings) {
        Ok(buf) => {
            if buf.is_empty() {
                println!("  Status: WARNING - synthesized empty buffer");
            } else {
                println!(
                    "  Status: OK - {} samples, {}Hz, {} channels",
                    buf.samples.len(),
                    buf.sample_rate,
                    buf.channels
                );
            }
        }
        Err(e) => {
            println!("  Status: FAILED - {}", e);
        }
    }
    println!();

    println!("[audio output]");
    match AudioStreams::new(SPEECH_MAX_DEPTH, TONE_MAX_DEPTH, SOUND_MAX_DEPTH) {
        Ok(streams) => {
            println!("  Audio device: OK");

            let tone_buf = ToneGenerator::generate(440.0, 200, 0.5);
            match streams.queue(StreamType::Tone, &tone_buf) {
                Ok(_) => println!("  Test tone (440Hz): playing..."),
                Err(e) => println!("  Test tone: FAILED - {}", e),
            }

            match engine.synthesize("Omnivox is ready.", &settings) {
                Ok(tts_buf) => {
                    let buf = tts_buffer_to_audio_buffer(tts_buf);
                    match streams.queue(StreamType::Speech, &buf) {
                        Ok(_) => println!("  Test speech: playing..."),
                        Err(e) => println!("  Test speech: FAILED - {}", e),
                    }
                }
                Err(e) => println!("  Test speech: FAILED - {}", e),
            }

            std::thread::sleep(std::time::Duration::from_secs(3));
            println!("  Playback: complete");
        }
        Err(e) => {
            println!("  Audio device: FAILED - {}", e);
            println!("  No audio output available. Check your sound device.");
        }
    }
    println!();

    println!("[sound files]");
    let loader = AudioFileLoader::with_cache();
    let test_paths = [
        "test-sounds/button.ogg",
        "test-sounds/complete.ogg",
    ];
    for path in &test_paths {
        let full = std::path::Path::new(path);
        if full.exists() {
            match loader.load(full) {
                Ok(buf) => println!("  {}: OK ({} samples)", path, buf.samples.len()),
                Err(e) => println!("  {}: FAILED - {}", path, e),
            }
        }
    }

    println!();
    println!("Diagnostic check complete. If you heard a tone and speech, everything is working.");
}

fn write_wav(path: &str, samples: &[f32], sample_rate: u32, channels: u16) -> Result<()> {
    let bytes_per_sample: u32 = 4;
    let data_size = samples.len() as u32 * bytes_per_sample;
    let mut f = std::fs::File::create(path)?;

    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_size).to_le_bytes())?;
    f.write_all(b"WAVE")?;

    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&3u16.to_le_bytes())?;
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&sample_rate.to_le_bytes())?;
    let byte_rate = sample_rate * channels as u32 * bytes_per_sample;
    f.write_all(&byte_rate.to_le_bytes())?;
    let block_align = channels * bytes_per_sample as u16;
    f.write_all(&block_align.to_le_bytes())?;
    f.write_all(&32u16.to_le_bytes())?;

    f.write_all(b"data")?;
    f.write_all(&data_size.to_le_bytes())?;
    for &s in samples {
        f.write_all(&s.to_le_bytes())?;
    }

    Ok(())
}

fn cmd_dump_wav(engine_name: &str, voice: &str, output: &str, text: &str) {
    let engine = match create_engine(engine_name) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to create TTS engine: {}", e);
            std::process::exit(1);
        }
    };

    let settings = TtsSettings {
        voice: voice.to_string(),
        rate: 0.5,
        pitch: 1.0,
        volume: 1.0,
    };

    println!("Voice: {}", voice);
    println!("Text: {}", text);
    println!("Rate: {}, Pitch: {}, Volume: {}", settings.rate, settings.pitch, settings.volume);

    let tts_buf = match engine.synthesize(text, &settings) {
        Ok(buf) => buf,
        Err(e) => {
            eprintln!("Synthesis failed: {}", e);
            std::process::exit(1);
        }
    };

    if tts_buf.is_empty() {
        eprintln!("Synthesis produced empty buffer");
        std::process::exit(1);
    }

    println!(
        "TTS output: {} samples, {}Hz, {}ch, {:.2}s",
        tts_buf.samples.len(),
        tts_buf.sample_rate,
        tts_buf.channels,
        tts_buf.duration()
    );

    let raw_path = output.replace(".wav", "_raw.wav");
    if let Err(e) = write_wav(&raw_path, &tts_buf.samples, tts_buf.sample_rate, tts_buf.channels) {
        eprintln!("Failed to write raw WAV: {}", e);
    } else {
        println!("Saved raw (post-resample, pre-pipeline): {}", raw_path);
    }

    let mut buf = tts_buffer_to_audio_buffer(tts_buf);
    let state = TtsState::default();
    let pipeline = build_speech_pipeline(&state, true, true);
    if let Err(e) = pipeline.process(&mut buf) {
        eprintln!("Pipeline processing failed: {}", e);
    }

    println!(
        "Pipeline output: {} samples, {:.2}s",
        buf.samples.len(),
        buf.samples.len() as f32 / (44100.0 * 2.0)
    );

    if let Err(e) = write_wav(output, &buf.samples, 44100, 2) {
        eprintln!("Failed to write processed WAV: {}", e);
    } else {
        println!("Saved processed (post-pipeline): {}", output);
    }

    println!("\nDone. Compare with reference WAVs from tools/tts_reference.swift");
}

fn cmd_play_wav(path: &str) {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to read {}: {}", path, e);
            std::process::exit(1);
        }
    };

    if data.len() < 44 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        eprintln!("Not a valid WAV file");
        std::process::exit(1);
    }

    let mut offset = 12;
    let mut audio_format: u16 = 0;
    let mut channels: u16 = 0;
    let mut sample_rate: u32 = 0;
    let mut bits_per_sample: u16 = 0;
    let mut pcm_data: &[u8] = &[];

    while offset + 8 <= data.len() {
        let chunk_id = &data[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
        ]) as usize;
        offset += 8;

        if chunk_id == b"fmt " && chunk_size >= 16 {
            audio_format = u16::from_le_bytes([data[offset], data[offset + 1]]);
            channels = u16::from_le_bytes([data[offset + 2], data[offset + 3]]);
            sample_rate = u32::from_le_bytes([
                data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
            ]);
            bits_per_sample = u16::from_le_bytes([data[offset + 14], data[offset + 15]]);
        } else if chunk_id == b"data" {
            pcm_data = &data[offset..offset + chunk_size.min(data.len() - offset)];
        }
        offset += chunk_size;
    }

    println!("Playing: {}", path);
    println!("Format: {}Hz, {}ch, {}bit, format={}", sample_rate, channels, bits_per_sample, audio_format);

    let samples: Vec<f32> = if audio_format == 3 && bits_per_sample == 32 {
        pcm_data.chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    } else if audio_format == 1 && bits_per_sample == 16 {
        pcm_data.chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect()
    } else {
        eprintln!("Unsupported format: audio_format={}, bits={}", audio_format, bits_per_sample);
        std::process::exit(1);
    };

    let duration_secs = samples.len() as f32 / (sample_rate as f32 * channels as f32);
    println!("Duration: {:.2}s ({} samples)", duration_secs, samples.len());

    let final_samples = if sample_rate == 44100 && channels == 2 {
        samples
    } else {
        let tts_buf = omnivox_tts::AudioBuffer::new(samples, sample_rate, channels);
        let standard = tts_buf.to_standard_format();
        standard.samples
    };

    let buf = AudioBuffer::new(final_samples);
    let streams = match AudioStreams::new(SPEECH_MAX_DEPTH, TONE_MAX_DEPTH, SOUND_MAX_DEPTH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open audio device: {}", e);
            std::process::exit(1);
        }
    };

    println!("Playing through rodio...");
    if let Err(e) = streams.queue(StreamType::Speech, &buf) {
        eprintln!("Playback error: {}", e);
        std::process::exit(1);
    }

    let wait = std::time::Duration::from_secs_f32(duration_secs + 0.5);
    std::thread::sleep(wait);
    println!("Done.");
}

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

struct CliArgs {
    engine: String,
    action: String,
    voice: Option<String>,
    rate: Option<f32>,
    pitch: Option<f32>,
    voice_volume: Option<f32>,
    tone_volume: Option<f32>,
    sound_volume: Option<f32>,
    audio_target: Option<String>,
}

fn parse_float_flag(flag: &str, args: &[String], i: &mut usize) -> f32 {
    *i += 1;
    if *i < args.len() {
        args[*i].parse::<f32>().unwrap_or_else(|_| {
            eprintln!("Error: {} requires a number", flag);
            std::process::exit(1);
        })
    } else {
        eprintln!("Error: {} requires a value", flag);
        std::process::exit(1);
    }
}

fn parse_string_flag(flag: &str, args: &[String], i: &mut usize) -> String {
    *i += 1;
    if *i < args.len() {
        args[*i].clone()
    } else {
        eprintln!("Error: {} requires a value", flag);
        std::process::exit(1);
    }
}

fn parse_args() -> CliArgs {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cli = CliArgs {
        engine: String::new(),
        action: String::from("server"),
        voice: None,
        rate: None,
        pitch: None,
        voice_volume: None,
        tone_volume: None,
        sound_volume: None,
        audio_target: None,
    };

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => cli.action = String::from("help"),
            "--version" | "-V" => cli.action = String::from("version"),
            "--check" => cli.action = String::from("check"),
            "--list-voices" => cli.action = String::from("list-voices"),
            "--list-voices-alist" => cli.action = String::from("list-voices-alist"),
            "--dump-wav" => cli.action = String::from("dump-wav"),
            "--play-wav" => cli.action = String::from("play-wav"),
            "--engine" => cli.engine = parse_string_flag("--engine", &args, &mut i),
            "--voice" => cli.voice = Some(parse_string_flag("--voice", &args, &mut i)),
            "--rate" => cli.rate = Some(parse_float_flag("--rate", &args, &mut i)),
            "--pitch" => cli.pitch = Some(parse_float_flag("--pitch", &args, &mut i)),
            "--voice-volume" => cli.voice_volume = Some(parse_float_flag("--voice-volume", &args, &mut i)),
            "--tone-volume" => cli.tone_volume = Some(parse_float_flag("--tone-volume", &args, &mut i)),
            "--sound-volume" => cli.sound_volume = Some(parse_float_flag("--sound-volume", &args, &mut i)),
            "--audio-target" => cli.audio_target = Some(parse_string_flag("--audio-target", &args, &mut i)),
            other => {
                if cli.action == "dump-wav" || cli.action == "play-wav" {
                    break;
                }
                eprintln!("Unknown option: {}", other);
                eprintln!("Try 'omnivox --help' for usage.");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    cli
}

fn apply_cli_flags(cli: &CliArgs, state: &mut TtsState) {
    if let Some(ref voice) = cli.voice {
        info!("Setting voice from flag: {}", voice);
        state.current_voice = voice.clone();
    }
    if let Some(rate) = cli.rate {
        info!("Setting speech rate from flag: {}", rate);
        state.speech_rate = rate;
    }
    if let Some(pitch) = cli.pitch {
        info!("Setting pitch from flag: {}", pitch);
        state.pitch_multiplier = pitch;
    }
    if let Some(vol) = cli.voice_volume {
        info!("Setting voice volume from flag: {}", vol);
        state.voice_volume = vol;
    }
    if let Some(vol) = cli.tone_volume {
        info!("Setting tone volume from flag: {}", vol);
        state.tone_volume = vol;
    }
    if let Some(vol) = cli.sound_volume {
        info!("Setting sound volume from flag: {}", vol);
        state.sound_volume = vol;
    }
    if let Some(ref target) = cli.audio_target {
        if let Some(channel_mode) = ChannelMode::parse(target) {
            info!("Setting audio target from flag: {}", target);
            state.speech_routing.channel_mode = channel_mode;
            state.notification_routing.channel_mode = channel_mode;
            state.tone_routing.channel_mode = channel_mode;
            state.sound_routing.channel_mode = channel_mode;
        } else {
            warn!("Invalid --audio-target value: {}", target);
        }
    }
}

// ---------------------------------------------------------------------------
// Synthesis worker
// ---------------------------------------------------------------------------

/// True if the request's generation stamp no longer matches the current counter.
/// Called before and after each blocking synthesis call.
#[inline(always)]
fn is_stale(request_gen: u64, gen_counter: &AtomicU64) -> bool {
    gen_counter.load(Ordering::Acquire) != request_gen
}

/// Shared context for all synthesis operations in the worker thread.
struct SynthCtx<'a> {
    gen: u64,
    gen_counter: &'a AtomicU64,
    engine: &'a dyn TtsEngine,
    control: &'a AudioControl,
}

impl SynthCtx<'_> {
    fn is_stale(&self) -> bool {
        is_stale(self.gen, self.gen_counter)
    }
}

/// Synthesize one text chunk and queue it on the speech stream.
/// Returns false if the request was cancelled before or during synthesis.
fn synthesize_chunk(
    chunk: &str,
    settings: &TtsSettings,
    state: &TtsState,
    is_first: bool,
    is_last: bool,
    ctx: &SynthCtx,
) -> bool {
    if ctx.is_stale() {
        return false;
    }

    match ctx.engine.synthesize(chunk, settings) {
        Ok(tts_buf) => {
            if ctx.is_stale() {
                return false;
            }
            let mut buf = tts_buffer_to_audio_buffer(tts_buf);
            let pipeline = build_speech_pipeline(state, is_first, is_last);
            if let Err(e) = pipeline.process(&mut buf) {
                warn!("Pipeline error: {}", e);
            }
            if let Err(e) = ctx.control.queue(StreamType::Speech, &buf) {
                warn!("Speech queue error: {}", e);
            }
            true
        }
        Err(e) => {
            warn!("Synthesis error: {}", e);
            true
        }
    }
}

/// Process a dispatched batch of queue items in the worker thread.
fn process_batch(
    items: Vec<QueueItem>,
    mut state: TtsState,
    ctx: &SynthCtx,
    loader: &AudioFileLoader,
) {
    if ctx.is_stale() {
        return;
    }

    // Pre-count speech chunks so we know first/last positions for padding.
    // This requires a pre-pass but avoids double-padding at voice boundaries.
    let total_speech_chunks: usize = items
        .iter()
        .map(|item| match item {
            QueueItem::Speech(text) => {
                let processed = preprocess_text(text, &state);
                chunk_text(&processed, 15).len()
            }
            _ => 0,
        })
        .sum();

    let mut speech_chunk_index: usize = 0;

    for item in items {
        if ctx.is_stale() {
            return;
        }

        match item {
            QueueItem::Speech(text) => {
                let settings = TtsSettings {
                    voice: state.current_voice.clone(),
                    rate: state.speech_rate,
                    pitch: state.pitch_multiplier,
                    volume: 1.0,
                };
                let processed = preprocess_text(&text, &state);
                let chunks = chunk_text(&processed, 15);

                for chunk in chunks {
                    let is_first = speech_chunk_index == 0;
                    let is_last = speech_chunk_index == total_speech_chunks - 1;
                    if !synthesize_chunk(&chunk, &settings, &state, is_first, is_last, ctx) {
                        return;
                    }
                    speech_chunk_index += 1;
                }
            }

            QueueItem::Code(codes) => {
                if let Some(voice) = extract_voice(&codes) {
                    debug!("Voice switch: {}", voice);
                    state.current_voice = voice;
                }
                if let Some(pitch_str) = extract_pitch(&codes) {
                    if let Ok(pitch) = pitch_str.parse::<f32>() {
                        state.pitch_multiplier = pitch;
                    }
                }
            }

            QueueItem::Tone { frequency, duration } => {
                let mut buf = ToneGenerator::generate(frequency as f32, duration, state.tone_volume);
                let pipeline = build_tone_pipeline(&state);
                if let Err(e) = pipeline.process(&mut buf) {
                    warn!("Tone pipeline error: {}", e);
                }
                if let Err(e) = ctx.control.queue(StreamType::Tone, &buf) {
                    warn!("Tone queue error: {}", e);
                }
            }

            QueueItem::Silence { duration } => {
                let buf = AudioBuffer::silence(duration as f32 / 1000.0);
                if let Err(e) = ctx.control.queue(StreamType::Speech, &buf) {
                    warn!("Silence queue error: {}", e);
                }
            }

            QueueItem::AudioIcon { path } => {
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
                    Err(e) => warn!("Failed to load audio icon {}: {}", path.display(), e),
                }
            }
        }
    }
}

/// The synthesis worker thread entry point.
///
/// Receives `SynthRequest`s from the reader thread via an mpsc channel and
/// processes them one at a time. Checks the generation counter before and after
/// each blocking `engine.synthesize()` call; stale requests are silently
/// discarded.
fn synthesis_worker(
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
                    if !synthesize_chunk(&chunk, &settings, &state, i == 0, i == count - 1, &ctx) {
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
                synthesize_chunk(&text.to_lowercase(), &settings, &letter_state, true, true, &ctx);
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
// Reader loop (main thread)
// ---------------------------------------------------------------------------

/// Interrupt: increment the generation counter, stop audio immediately, and
/// optionally stop the TTS engine's current synthesis call.
///
/// `stop_engine` should be `true` only for hard stops (the `s` command).
/// For `tts_say` and `letter`, pass `false` — the generation counter already
/// causes the worker to discard stale results, and calling `engine.stop()`
/// cross-thread while AVSpeechSynthesizer is running on its GCD queue
/// corrupts the synthesizer state for subsequent calls.
fn interrupt(
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

/// Reader loop: processes stdin commands and drives the synthesis worker.
///
/// Does NOT own `AudioStreams` — the caller keeps it alive (and drops it after
/// this returns) so the `OutputStream` drop guard outlives playback.
/// Drain is done here via `control.drain()` before returning.
fn run_server(
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

        handle_command(
            command,
            &mut state,
            &mut pending,
            &mut current_gen,
            &gen_counter,
            &engine,
            &control,
            &tx,
        );
    }

    info!("Stdin closed; waiting for synthesis worker to finish");
    drop(tx);
    let _ = worker_handle.join();

    info!("Draining audio output");
    control.drain();

    info!("Shutting down");
    Ok(())
}

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
                    if let (Ok(freq), Ok(dur)) =
                        (parts[0].parse::<u32>(), parts[1].parse::<u32>())
                    {
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
                let expanded = expand_tilde(&path);
                debug!("Queue audio icon: {}", expanded.display());
                pending.push(QueueItem::AudioIcon { path: expanded });
            }
        }

        // --- Dispatch: send accumulated items to worker ---

        CommandId::Dispatch => {
            if !pending.is_empty() {
                debug!("Dispatch {} items (gen={})", pending.len(), current_gen);
                let items = mem::take(pending);
                let _ = tx.send(SynthRequest::Batch {
                    items,
                    state: state.clone(),
                    gen: *current_gen,
                });
            }
        }

        // --- Interrupting commands (take effect on reader thread immediately) ---

        CommandId::Stop => {
            debug!("Stop");
            interrupt(current_gen, gen_counter, control, engine.as_ref(), false, true);
            pending.clear();
        }

        CommandId::TtsSay => {
            if let Some(text) = command.args {
                debug!("tts_say: {}", text);
                // Interrupt speech stream only; tones/sounds continue.
                // Do NOT call engine.stop() here — the generation counter discards
                // stale results; cross-thread AVSpeech stop corrupts synthesizer state.
                interrupt(current_gen, gen_counter, control, engine.as_ref(), true, false);
                let _ = tx.send(SynthRequest::Immediate {
                    text,
                    state: state.clone(),
                    gen: *current_gen,
                });
            }
        }

        CommandId::Letter => {
            if let Some(letter) = command.args {
                debug!("Letter: {}", letter);
                interrupt(current_gen, gen_counter, control, engine.as_ref(), true, false);
                let _ = tx.send(SynthRequest::Letter {
                    text: letter,
                    state: state.clone(),
                    gen: *current_gen,
                });
            }
        }

        CommandId::PlaySound => {
            if let Some(path) = command.args {
                let expanded = expand_tilde(&path);
                debug!("Play sound: {}", expanded.display());
                // Sound plays concurrently; no speech interruption
                let _ = tx.send(SynthRequest::PlaySound {
                    path: expanded,
                    state: state.clone(),
                    gen: *current_gen,
                });
            }
        }

        CommandId::Version => {
            let version_text = format!("Omnivox version {}", VERSION.replace('.', " dot "));
            let _ = tx.send(SynthRequest::Immediate {
                text: version_text,
                state: state.clone(),
                gen: *current_gen,
            });
        }

        // --- State management (reader thread only, instant) ---

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

        _ => {
            debug!("Command not yet implemented: {:?}", command.id);
        }
    }
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

fn insert_space_before_uppercase(input: &str) -> String {
    let mut result = String::with_capacity(input.len() * 2);
    for c in input.chars() {
        if c.is_uppercase() && !result.is_empty() {
            result.push(' ');
        }
        result.push(c);
    }
    result
}

fn extract_voice(codes: &str) -> Option<String> {
    VOICE_RE.captures(codes)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

fn extract_pitch(codes: &str) -> Option<String> {
    PITCH_RE.captures(codes)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = parse_args();

    match cli.action.as_str() {
        "help" => {
            print_help();
            return Ok(());
        }
        "version" => {
            print_version();
            return Ok(());
        }
        "check" => {
            cmd_check(&cli.engine);
            return Ok(());
        }
        "list-voices" => {
            let engine = create_engine(&cli.engine)?;
            cmd_list_voices(engine.as_ref());
            return Ok(());
        }
        "list-voices-alist" => {
            let engine = create_engine(&cli.engine)?;
            cmd_list_voices_alist(engine.as_ref());
            return Ok(());
        }
        "play-wav" => {
            let remaining: Vec<String> = std::env::args().collect();
            let idx = remaining.iter().position(|a| a == "--play-wav").unwrap_or(0);
            if idx + 1 >= remaining.len() {
                eprintln!("Usage: omnivox --play-wav <file.wav>");
                std::process::exit(1);
            }
            cmd_play_wav(&remaining[idx + 1]);
            return Ok(());
        }
        "dump-wav" => {
            let remaining: Vec<String> = std::env::args().collect();
            let dump_idx = remaining.iter().position(|a| a == "--dump-wav").unwrap_or(0);
            let dump_args: Vec<&str> = remaining[dump_idx + 1..].iter().map(|s| s.as_str()).collect();
            if dump_args.len() < 2 {
                eprintln!("Usage: omnivox --dump-wav <voice> <output.wav> [text...]");
                eprintln!("  voice: e.g. 'en-US:Alex', 'en-US:Samantha (Enhanced)', 'en-US'");
                eprintln!("  Example: omnivox --dump-wav 'en-US:Alex' alex.wav Hello world");
                std::process::exit(1);
            }
            let voice = dump_args[0];
            let output = dump_args[1];
            let text = if dump_args.len() > 2 {
                dump_args[2..].join(" ")
            } else {
                "The quick brown fox jumps over the lazy dog".to_string()
            };
            cmd_dump_wav(&cli.engine, voice, output, &text);
            return Ok(());
        }
        _ => {}
    }

    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    info!("Omnivox v{} starting", VERSION);

    let engine = create_engine(&cli.engine)?;
    info!("TTS engine initialized");

    let voices = engine.available_voices();
    info!("Found {} voices", voices.len());

    let streams = AudioStreams::new(SPEECH_MAX_DEPTH, TONE_MAX_DEPTH, SOUND_MAX_DEPTH)
        .map_err(|e| anyhow::anyhow!("Audio streams init failed: {}", e))?;
    let control = streams.control();

    let mut state = TtsState::default();
    apply_audio_target_env(&mut state);
    apply_cli_flags(&cli, &mut state);

    let (tx, rx) = mpsc::channel::<SynthRequest>();
    let gen_counter = Arc::new(AtomicU64::new(0));

    // Spawn synthesis worker -- owns the channel receiver, engine clone, control clone
    let worker_handle = {
        let worker_engine = engine.clone();
        let worker_control = control.clone();
        let worker_gen = gen_counter.clone();
        let loader = AudioFileLoader::with_cache();
        std::thread::Builder::new()
            .name("omnivox-synth".to_string())
            .spawn(move || {
                synthesis_worker(rx, worker_gen, worker_engine, worker_control, loader);
            })
            .expect("Failed to spawn synthesis worker thread")
    };

    // On macOS, AVSpeechSynthesizer.writeUtterance:toBufferCallback: internally
    // uses the main GCD queue. If the main thread is blocked on stdin instead of
    // running a NSRunLoop, synthesis deadlocks. Fix: run the reader/server on a
    // background thread; main thread keeps AudioStreams alive and pumps the
    // NSRunLoop.
    //
    // AudioStreams is !Send (OutputStream is !Send on CPAL/CoreAudio), so it
    // stays here on the main thread. run_server() only needs AudioControl
    // (Send+Sync) for drain — AudioStreams is dropped here after run_server
    // returns, keeping the OutputStream alive until all audio has played.
    #[cfg(target_os = "macos")]
    {
        use std::sync::Mutex;
        let result: Arc<Mutex<Option<Result<()>>>> = Arc::new(Mutex::new(None));
        let result2 = result.clone();
        std::thread::Builder::new()
            .name("omnivox-reader".to_string())
            .spawn(move || {
                let r = run_server(engine, state, tx, control, gen_counter, worker_handle);
                *result2.lock().unwrap() = Some(r);
                omnivox_tts::macos::stop_main_runloop();
            })
            .expect("Failed to spawn reader thread");
        // Block main thread in NSRunLoop — required for AVSpeechSynthesizer.
        // Returns when stop_main_runloop() is called above.
        omnivox_tts::macos::run_main_runloop();
        // streams (AudioStreams / OutputStream) dropped here, after all audio played.
        drop(streams);
        return result.lock().unwrap().take().unwrap_or(Ok(()));
    }

    #[cfg(not(target_os = "macos"))]
    run_server(engine, state, tx, control, gen_counter, worker_handle)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_space_before_uppercase() {
        assert_eq!(insert_space_before_uppercase("helloWorld"), "hello World");
        assert_eq!(insert_space_before_uppercase("HTTPServer"), "H T T P Server");
        assert_eq!(insert_space_before_uppercase("lowercase"), "lowercase");
    }

    #[test]
    fn test_extract_voice() {
        assert_eq!(
            extract_voice("[{voice en-US:Samantha}]"),
            Some("en-US:Samantha".to_string())
        );
        assert_eq!(
            extract_voice("[{voice en-GB:Daniel}]"),
            Some("en-GB:Daniel".to_string())
        );
        assert_eq!(extract_voice("no voice here"), None);
    }

    #[test]
    fn test_extract_pitch() {
        assert_eq!(extract_pitch("[[pitch 1.5]]"), Some("1.5".to_string()));
        assert_eq!(extract_pitch("[[pitch 0.8]]"), Some("0.8".to_string()));
        assert_eq!(extract_pitch("no pitch here"), None);
    }

    #[test]
    fn test_tts_buffer_to_audio_buffer() {
        let tts_buf = omnivox_tts::AudioBuffer::new(vec![0.1, -0.1, 0.2, -0.2], 44100, 2);
        let audio_buf = tts_buffer_to_audio_buffer(tts_buf);
        assert_eq!(audio_buf.samples, vec![0.1, -0.1, 0.2, -0.2]);
        assert_eq!(audio_buf.frame_count(), 2);
    }

    #[test]
    fn test_tts_buffer_to_audio_buffer_empty() {
        let tts_buf = omnivox_tts::AudioBuffer::empty();
        let audio_buf = tts_buffer_to_audio_buffer(tts_buf);
        assert!(audio_buf.is_empty());
    }

    #[test]
    fn test_chunk_text_short() {
        let chunks = chunk_text("hello world", 15);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "hello world");
    }

    #[test]
    fn test_chunk_text_long() {
        let text = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen";
        let chunks = chunk_text(text, 15);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].split_whitespace().count(), 15);
        assert_eq!(chunks[1].split_whitespace().count(), 1);
    }

    #[test]
    fn test_apply_punctuation_none() {
        let result = apply_punctuation("hello $100", PunctuationLevel::None);
        assert_eq!(result, "hello  dollar 100");
    }

    #[test]
    fn test_apply_punctuation_some() {
        let result = apply_punctuation("a+b", PunctuationLevel::Some);
        assert_eq!(result, "a plus b");
    }

    #[test]
    fn test_apply_punctuation_all() {
        let result = apply_punctuation("a.b", PunctuationLevel::All);
        assert_eq!(result, "a dot b");
    }

    #[test]
    fn test_normalize_rate_normal() {
        assert!((normalize_rate(0.5) - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_normalize_rate_integer_scale() {
        assert!((normalize_rate(50.0) - 0.5).abs() < 0.001);
        assert!((normalize_rate(100.0) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_normalize_rate_clamp() {
        assert_eq!(normalize_rate(-1.0), 0.0);
        assert!((normalize_rate(200.0) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_is_stale() {
        let counter = AtomicU64::new(5);
        assert!(!is_stale(5, &counter));
        assert!(is_stale(4, &counter));
        assert!(is_stale(6, &counter));
    }

    #[test]
    fn test_expand_tilde_no_tilde() {
        let p = expand_tilde("/absolute/path");
        assert_eq!(p, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn test_rate_scaled_padding() {
        let slow = rate_scaled_padding(0.0);
        let fast = rate_scaled_padding(1.0);
        assert!(slow > fast);
        assert!(slow <= 0.02);
        assert!(fast >= 0.001);
    }
}
