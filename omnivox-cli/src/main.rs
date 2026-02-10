//! Omnivox CLI - Emacspeak Speech Server
//!
//! Cross-platform text-to-speech server implementing the Emacspeak protocol.
//! Uses a buffer-based audio pipeline: TTS/tone/file -> pipeline -> output.
//!
//! Audio is played on three concurrent streams (speech, tones, sounds).
//! Items within each stream serialize; different streams overlap.

use anyhow::Result;
use omnivox_audio::{
    AudioBuffer, AudioFileLoader, AudioPipeline, AudioStreams, ChannelRouter, SilenceTrimmer,
    StreamType, ToneGenerator, VolumeAdjust,
};
use omnivox_core::{
    parse_command,
    state::{ChannelMode, PunctuationLevel},
    Command, CommandId, CommandQueue, QueueItem, TtsState,
};
use omnivox_tts::espeak::EspeakTtsEngine;
#[cfg(target_os = "macos")]
use omnivox_tts::macos::MacOsTtsEngine;
#[cfg(target_os = "windows")]
use omnivox_tts::windows::WindowsTtsEngine;
use omnivox_tts::{TtsEngine, TtsSettings};
use std::io::{self, BufRead};
use tracing::{debug, error, info, warn};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Maximum queued items per audio stream before overflow drops old items.
/// Low values improve responsiveness for accessibility - old speech drops quickly.
const SPEECH_MAX_DEPTH: usize = 2;  // Was 10 - reduced for faster response
const TONE_MAX_DEPTH: usize = 2;     // Was 3 - keep tones responsive too
const SOUND_MAX_DEPTH: usize = 3;    // Was 5 - icons should be immediate

/// Convert a TTS AudioBuffer (omnivox_tts::AudioBuffer) to the pipeline
/// AudioBuffer (omnivox_audio::AudioBuffer). The TTS engine already outputs
/// stereo f32 at 44100Hz via to_standard_format(), so we just transfer samples.
fn tts_buffer_to_audio_buffer(tts_buf: omnivox_tts::AudioBuffer) -> AudioBuffer {
    if tts_buf.is_empty() {
        return AudioBuffer::empty();
    }
    AudioBuffer::new(tts_buf.samples)
}

/// Replace punctuation characters with their spoken names based on the
/// current punctuation level.
///
/// - None: only $ and %
/// - Some: $, #, -, ", (, ), *, ;, :, <, >, \, /, +, =, ~, `, !, ^
/// - All: all of the above plus @, _, ', ., ,
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

/// Get the user's home directory, checking platform-appropriate env vars.
fn home_dir() -> Option<std::ffi::OsString> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
}

/// Expand ~ to the user's home directory in paths.
fn expand_tilde(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return std::path::PathBuf::from(home).join(rest);
        }
    } else if path == "~" {
        if let Some(home) = home_dir() {
            return std::path::PathBuf::from(home);
        }
    }
    std::path::PathBuf::from(path)
}

/// Process text before synthesis: apply punctuation and split caps.
fn preprocess_text(text: &str, state: &TtsState) -> String {
    let mut processed = apply_punctuation(text, state.punctuation_level);
    if state.split_caps {
        processed = insert_space_before_uppercase(&processed);
    }
    processed
}

/// Apply OMNIVOX_AUDIO_TARGET environment variable to TTS state channel routing.
/// Valid values: "left", "right", "both" (default if not set or invalid).
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

/// Apply volume environment variables to TTS state.
/// Valid values: 0.0 to 1.0 (1.0 = 100% volume).
fn apply_volume_env(state: &mut TtsState) {
    if let Ok(vol) = std::env::var("OMNIVOX_VOICE_VOLUME") {
        if let Ok(v) = vol.parse::<f32>() {
            if (0.0..=1.0).contains(&v) {
                info!("Setting voice volume from env: {}", v);
                state.voice_volume = v;
            } else {
                warn!("Invalid OMNIVOX_VOICE_VOLUME value: {} (must be 0.0-1.0)", vol);
            }
        }
    }

    if let Ok(vol) = std::env::var("OMNIVOX_TONE_VOLUME") {
        if let Ok(v) = vol.parse::<f32>() {
            if (0.0..=1.0).contains(&v) {
                info!("Setting tone volume from env: {}", v);
                state.tone_volume = v;
            } else {
                warn!("Invalid OMNIVOX_TONE_VOLUME value: {} (must be 0.0-1.0)", vol);
            }
        }
    }

    if let Ok(vol) = std::env::var("OMNIVOX_SOUND_VOLUME") {
        if let Ok(v) = vol.parse::<f32>() {
            if (0.0..=1.0).contains(&v) {
                info!("Setting sound volume from env: {}", v);
                state.sound_volume = v;
            } else {
                warn!("Invalid OMNIVOX_SOUND_VOLUME value: {} (must be 0.0-1.0)", vol);
            }
        }
    }
}

/// Build a pipeline for speech audio based on current TTS state.
fn build_speech_pipeline(state: &TtsState) -> AudioPipeline {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(SilenceTrimmer::new()));
    pipeline.push(Box::new(VolumeAdjust::new(state.voice_volume)));
    pipeline.push(Box::new(ChannelRouter::new(
        state.speech_routing.channel_mode,
    )));
    pipeline
}

/// Build a pipeline for tone audio based on current TTS state.
fn build_tone_pipeline(state: &TtsState) -> AudioPipeline {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(VolumeAdjust::new(state.tone_volume)));
    pipeline.push(Box::new(ChannelRouter::new(
        state.tone_routing.channel_mode,
    )));
    pipeline
}

/// Build a pipeline for sound/audio icon playback based on current TTS state.
fn build_sound_pipeline(state: &TtsState) -> AudioPipeline {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(VolumeAdjust::new(state.sound_volume)));
    pipeline.push(Box::new(ChannelRouter::new(
        state.sound_routing.channel_mode,
    )));
    pipeline
}

/// Create the best available TTS engine: platform-native first, espeak-ng fallback.
/// `engine_name` overrides env var OMNIVOX_ENGINE. Values: "espeak", "native", or empty for auto.
fn create_engine(engine_name: &str) -> Result<Box<dyn TtsEngine>> {
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
                    return Ok(Box::new(engine));
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
                    return Ok(Box::new(engine));
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
            Ok(Box::new(engine))
        }
        Err(e) => {
            anyhow::bail!("No TTS engine available: {}", e);
        }
    }
}

/// Return the name of the platform-native engine for display purposes.
fn native_engine_name() -> &'static str {
    #[cfg(target_os = "macos")]
    { "macos (AVSpeechSynthesizer)" }
    #[cfg(target_os = "windows")]
    { "winrt (Windows SpeechSynthesizer)" }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { "none (espeak-ng is the only backend)" }
}

/// Print help text and exit.
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
    println!("    --engine NAME    Select TTS engine: native, espeak");
    println!();
    println!("ENGINES:");
    println!("    native    Platform-native TTS: {}", native);
    println!("    espeak    espeak-ng (cross-platform, always available)");
    println!();
    println!("Without options, starts the Emacspeak protocol server on stdin.");
    println!();
    println!("ENVIRONMENT:");
    println!("    OMNIVOX_ENGINE    Set to 'espeak' to force espeak-ng engine");
    println!();
    println!("EMACSPEAK SETUP:");
    println!("    (setq dtk-program \"omnivox\")");
    println!("    Ensure omnivox is in your PATH or in emacspeak/servers/");
}

/// Print version and exit.
fn print_version() {
    println!("omnivox {}", VERSION);
}

/// List all available voices, grouped by language.
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

/// Run diagnostic self-test: check engine, voices, synthesis, tones, and audio output.
fn cmd_check(engine_name: &str) {
    println!("Omnivox v{} diagnostic check", VERSION);
    println!("=============================\n");

    // Platform
    println!("[platform]");
    println!("  OS: {}", std::env::consts::OS);
    println!("  Arch: {}", std::env::consts::ARCH);
    println!("  Native engine: {}", native_engine_name());
    println!();

    // Home directory
    println!("[home]");
    match home_dir() {
        Some(h) => println!("  Home: {}", std::path::Path::new(&h).display()),
        None => println!("  WARNING: Could not determine home directory (HOME/USERPROFILE not set)"),
    }
    println!();

    // Engine
    println!("[engine]");
    let engine: Box<dyn TtsEngine> = match create_engine(engine_name) {
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

    // Voices
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

    // Synthesis test
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

    // Audio output test
    println!("[audio output]");
    match AudioStreams::new(SPEECH_MAX_DEPTH, TONE_MAX_DEPTH, SOUND_MAX_DEPTH) {
        Ok(streams) => {
            println!("  Audio device: OK");

            // Play a short test tone
            let tone_buf = ToneGenerator::generate(440.0, 200, 0.5);
            match streams.queue(StreamType::Tone, &tone_buf) {
                Ok(_) => println!("  Test tone (440Hz): playing..."),
                Err(e) => println!("  Test tone: FAILED - {}", e),
            }

            // Synthesize and play test speech
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

            // Wait for audio to finish
            std::thread::sleep(std::time::Duration::from_secs(3));
            println!("  Playback: complete");
        }
        Err(e) => {
            println!("  Audio device: FAILED - {}", e);
            println!("  No audio output available. Check your sound device.");
        }
    }
    println!();

    // Sound file loading test
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

/// Parse CLI arguments. Returns (engine_name, action) where action is
/// "server" (default), "help", "version", "check", or "list-voices".
fn parse_args() -> (String, String) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut engine = String::new();
    let mut action = String::from("server");

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => action = String::from("help"),
            "--version" | "-V" => action = String::from("version"),
            "--check" => action = String::from("check"),
            "--list-voices" => action = String::from("list-voices"),
            "--engine" => {
                i += 1;
                if i < args.len() {
                    engine = args[i].clone();
                } else {
                    eprintln!("Error: --engine requires a value (native, espeak)");
                    std::process::exit(1);
                }
            }
            other => {
                eprintln!("Unknown option: {}", other);
                eprintln!("Try 'omnivox --help' for usage.");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    (engine, action)
}

#[tokio::main]
async fn main() -> Result<()> {
    let (engine_name, action) = parse_args();

    // Handle non-server actions before initializing tracing (they print to stdout)
    match action.as_str() {
        "help" => {
            print_help();
            return Ok(());
        }
        "version" => {
            print_version();
            return Ok(());
        }
        "check" => {
            cmd_check(&engine_name);
            return Ok(());
        }
        "list-voices" => {
            let engine = create_engine(&engine_name)?;
            cmd_list_voices(engine.as_ref());
            return Ok(());
        }
        _ => {} // "server" - continue below
    }

    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    info!("Omnivox v{} starting", VERSION);

    let engine = create_engine(&engine_name)?;
    info!("TTS engine initialized");

    let voices = engine.available_voices();
    info!("Found {} voices", voices.len());

    let streams = AudioStreams::new(SPEECH_MAX_DEPTH, TONE_MAX_DEPTH, SOUND_MAX_DEPTH)
        .map_err(|e| anyhow::anyhow!("Audio streams init failed: {}", e))?;
    let loader = AudioFileLoader::with_cache();

    let mut state = TtsState::default();
    apply_audio_target_env(&mut state);
    apply_volume_env(&mut state);
    let mut queue = CommandQueue::new();

    // Speak version (non-blocking -- server is ready for commands immediately)
    let settings = TtsSettings::default();
    let version_text = format!("Omnivox version {}", VERSION.replace('.', " dot "));
    if let Ok(tts_buf) = engine.synthesize(&version_text, &settings) {
        let mut buf = tts_buffer_to_audio_buffer(tts_buf);
        let pipeline = build_speech_pipeline(&state);
        let _ = pipeline.process(&mut buf);
        let _ = streams.queue(StreamType::Speech, &buf);
    }

    info!("Ready to accept commands from stdin");

    let stdin = io::stdin();
    let reader = stdin.lock();

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        debug!("Received command: {}", line);

        match parse_command(line) {
            Ok(command) => {
                if let Err(e) = process_command(
                    command,
                    &mut state,
                    &mut queue,
                    engine.as_ref(),
                    &streams,
                    &loader,
                )
                .await
                {
                    error!("Error processing command: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to parse command '{}': {}", line, e);
            }
        }
    }

    info!("Shutting down");
    Ok(())
}

/// Process a parsed command
async fn process_command(
    command: Command,
    state: &mut TtsState,
    queue: &mut CommandQueue,
    engine: &dyn TtsEngine,
    streams: &AudioStreams,
    loader: &AudioFileLoader,
) -> Result<()> {
    match command.id {
        // Queue commands
        CommandId::Queue => {
            if let Some(text) = command.args {
                debug!("Queuing speech: {}", text);
                queue.enqueue(QueueItem::Speech(text));
            }
        }

        CommandId::Code => {
            if let Some(codes) = command.args {
                debug!("Queuing codes: {}", codes);
                queue.enqueue(QueueItem::Code(codes));
            }
        }

        CommandId::Tone => {
            if let Some(args) = command.args {
                let parts: Vec<&str> = args.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let (Ok(freq), Ok(dur)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                        debug!("Queuing tone: {}Hz for {}ms", freq, dur);
                        queue.enqueue(QueueItem::Tone {
                            frequency: freq,
                            duration: dur,
                        });
                    }
                }
            }
        }

        CommandId::Silence => {
            if let Some(duration) = command.args {
                if let Ok(dur) = duration.parse::<u32>() {
                    debug!("Queuing silence: {}ms", dur);
                    queue.enqueue(QueueItem::Silence { duration: dur });
                }
            }
        }

        CommandId::AudioIcon => {
            if let Some(path) = command.args {
                let expanded = expand_tilde(&path);
                debug!("Queuing audio icon: {}", expanded.display());
                queue.enqueue(QueueItem::AudioIcon {
                    path: expanded,
                });
            }
        }

        // Dispatch queue
        CommandId::Dispatch => {
            debug!("Dispatching queue ({} items)", queue.len());
            let items = queue.dispatch();
            process_queue_items(items, state, engine, streams, loader).await?;
        }

        // Immediate commands
        CommandId::Stop => {
            debug!("Stopping all audio");
            streams.stop_all();
            engine.stop();
            queue.clear();
        }

        CommandId::Letter => {
            if let Some(letter) = command.args {
                debug!("Speaking letter: {}", letter);
                // Letters interrupt current speech
                streams.stop(StreamType::Speech);

                let old_rate = state.speech_rate;
                let old_pitch = state.pitch_multiplier;

                state.speech_rate = state.character_rate();

                if letter.chars().next().is_some_and(|c| c.is_uppercase()) {
                    if state.allcaps_beep {
                        // Play a short beep for capital letters (on tone stream, concurrent)
                        let mut tone_buf = ToneGenerator::generate(440.0, 10, state.tone_volume);
                        let pipeline = build_tone_pipeline(state);
                        let _ = pipeline.process(&mut tone_buf);
                        let _ = streams.queue(StreamType::Tone, &tone_buf);
                    } else {
                        state.pitch_multiplier = 1.5;
                    }
                }

                let settings = TtsSettings {
                    voice: state.current_voice.clone(),
                    rate: state.speech_rate,
                    pitch: state.pitch_multiplier,
                    volume: 1.0, // Volume applied in pipeline, not here
                };

                if let Ok(tts_buf) = engine.synthesize(&letter.to_lowercase(), &settings) {
                    let mut buf = tts_buffer_to_audio_buffer(tts_buf);
                    let pipeline = build_speech_pipeline(state);
                    let _ = pipeline.process(&mut buf);
                    let _ = streams.queue(StreamType::Speech, &buf);
                }

                state.speech_rate = old_rate;
                state.pitch_multiplier = old_pitch;
            }
        }

        CommandId::TtsSay => {
            if let Some(text) = command.args {
                let processed_text = preprocess_text(&text, state);
                debug!("Speaking immediately: {}", processed_text);
                // Immediate speech interrupts current speech stream
                streams.stop(StreamType::Speech);
                engine.stop();
                let settings = TtsSettings {
                    voice: state.current_voice.clone(),
                    rate: state.speech_rate,
                    pitch: state.pitch_multiplier,
                    volume: 1.0, // Volume applied in pipeline, not here
                };
                if let Ok(tts_buf) = engine.synthesize(&processed_text, &settings) {
                    let mut buf = tts_buffer_to_audio_buffer(tts_buf);
                    let pipeline = build_speech_pipeline(state);
                    let _ = pipeline.process(&mut buf);
                    let _ = streams.queue(StreamType::Speech, &buf);
                }
            }
        }

        CommandId::PlaySound => {
            if let Some(path) = command.args {
                let expanded = expand_tilde(&path);
                debug!("Playing sound immediately: {}", expanded.display());
                // Sounds play concurrently on the sound stream
                match loader.load(&expanded) {
                    Ok(mut buf) => {
                        let pipeline = build_sound_pipeline(state);
                        let _ = pipeline.process(&mut buf);
                        let _ = streams.queue(StreamType::Sound, &buf);
                    }
                    Err(e) => {
                        warn!("Failed to load audio file {}: {}", expanded.display(), e);
                    }
                }
            }
        }

        CommandId::Version => {
            info!("Speaking version");
            let version_text = format!("Omnivox version {}", VERSION.replace('.', " dot "));
            let settings = TtsSettings::default();
            if let Ok(tts_buf) = engine.synthesize(&version_text, &settings) {
                let mut buf = tts_buffer_to_audio_buffer(tts_buf);
                let pipeline = build_speech_pipeline(state);
                let _ = pipeline.process(&mut buf);
                let _ = streams.queue(StreamType::Speech, &buf);
            }
        }

        // State management
        CommandId::TtsSetSpeechRate => {
            if let Some(rate) = command.args {
                if let Ok(r) = rate.parse::<f32>() {
                    debug!("Setting speech rate: {}", r);
                    state.speech_rate = r;
                }
            }
        }

        CommandId::TtsSetVoice => {
            if let Some(voice) = command.args {
                debug!("Setting voice: {}", voice);
                state.current_voice = voice;
            }
        }

        CommandId::TtsSetPitchMultiplier => {
            if let Some(pitch) = command.args {
                if let Ok(p) = pitch.parse::<f32>() {
                    debug!("Setting pitch multiplier: {}", p);
                    state.pitch_multiplier = p;
                }
            }
        }

        CommandId::TtsSetVoiceVolume => {
            if let Some(vol) = command.args {
                if let Ok(v) = vol.parse::<f32>() {
                    debug!("Setting voice volume: {}", v);
                    state.voice_volume = v;
                }
            }
        }

        CommandId::TtsSetToneVolume => {
            if let Some(vol) = command.args {
                if let Ok(v) = vol.parse::<f32>() {
                    debug!("Setting tone volume: {}", v);
                    state.tone_volume = v;
                }
            }
        }

        CommandId::TtsSetSoundVolume => {
            if let Some(vol) = command.args {
                if let Ok(v) = vol.parse::<f32>() {
                    debug!("Setting sound volume: {}", v);
                    state.sound_volume = v;
                }
            }
        }

        CommandId::TtsSetCharacterScale => {
            if let Some(scale) = command.args {
                if let Ok(s) = scale.parse::<f32>() {
                    debug!("Setting character scale: {}", s);
                    state.character_scale = s;
                }
            }
        }

        CommandId::TtsSplitCaps => {
            if let Some(flag) = command.args {
                state.split_caps = flag == "1";
                debug!("Split caps: {}", state.split_caps);
            }
        }

        CommandId::TtsAllCapsBeep => {
            if let Some(flag) = command.args {
                state.allcaps_beep = flag == "1";
                debug!("All caps beep: {}", state.allcaps_beep);
            }
        }

        CommandId::TtsSetPunctuations => {
            if let Some(level) = command.args {
                if let Some(punct) = omnivox_core::state::PunctuationLevel::parse(&level) {
                    debug!("Setting punctuation level: {:?}", punct);
                    state.punctuation_level = punct;
                }
            }
        }

        CommandId::TtsSyncState => {
            if let Some(args) = command.args {
                debug!("Syncing state: {}", args);
                let parts: Vec<&str> = args.split_whitespace().collect();
                // Format: punctuations split_caps allcaps_beep rate
                if parts.len() >= 4 {
                    if let Some(punct) = omnivox_core::state::PunctuationLevel::parse(parts[0]) {
                        state.punctuation_level = punct;
                    }
                    state.split_caps = parts[1] == "1";
                    state.allcaps_beep = parts[2] == "1";
                    if let Ok(r) = parts[3].parse::<f32>() {
                        state.speech_rate = r;
                    }
                }
            }
        }

        CommandId::TtsReset => {
            debug!("Resetting state");
            streams.stop_all();
            engine.stop();
            state.reset();
            queue.clear();
        }

        CommandId::TtsExit => {
            info!("Exit command received");
            std::process::exit(0);
        }

        _ => {
            debug!("Command not yet implemented: {:?}", command.id);
        }
    }

    Ok(())
}

/// Process queue items after dispatch.
///
/// Speech items serialize on the speech stream. Tones and audio icons play
/// concurrently on their own streams. No waiting between items.
async fn process_queue_items(
    items: Vec<QueueItem>,
    state: &mut TtsState,
    engine: &dyn TtsEngine,
    streams: &AudioStreams,
    loader: &AudioFileLoader,
) -> Result<()> {
    for item in items {
        match item {
            QueueItem::Speech(text) => {
                let settings = TtsSettings {
                    voice: state.current_voice.clone(),
                    rate: state.speech_rate,
                    pitch: state.pitch_multiplier,
                    volume: 1.0, // Volume applied in pipeline, not here
                };

                let processed_text = preprocess_text(&text, state);

                debug!("Speaking queued text: {}", processed_text);
                match engine.synthesize(&processed_text, &settings) {
                    Ok(tts_buf) => {
                        let mut buf = tts_buffer_to_audio_buffer(tts_buf);
                        let pipeline = build_speech_pipeline(state);
                        if let Err(e) = pipeline.process(&mut buf) {
                            warn!("Pipeline error: {}", e);
                        }
                        if let Err(e) = streams.queue(StreamType::Speech, &buf) {
                            warn!("Speech queue error: {}", e);
                        }
                    }
                    Err(e) => warn!("Synthesis error: {}", e),
                }
            }

            QueueItem::Code(codes) => {
                debug!("Processing codes: {}", codes);
                if let Some(voice) = extract_voice(&codes) {
                    debug!("Switching voice to: {}", voice);
                    state.current_voice = voice;
                }
                if let Some(pitch_str) = extract_pitch(&codes) {
                    if let Ok(pitch) = pitch_str.parse::<f32>() {
                        debug!("Switching pitch to: {}", pitch);
                        state.pitch_multiplier = pitch;
                    }
                }
            }

            QueueItem::Tone { frequency, duration } => {
                debug!("Playing tone: {}Hz for {}ms", frequency, duration);
                let mut buf =
                    ToneGenerator::generate(frequency as f32, duration, state.tone_volume);
                let pipeline = build_tone_pipeline(state);
                if let Err(e) = pipeline.process(&mut buf) {
                    warn!("Pipeline error: {}", e);
                }
                if let Err(e) = streams.queue(StreamType::Tone, &buf) {
                    warn!("Tone queue error: {}", e);
                }
            }

            QueueItem::Silence { duration } => {
                debug!("Silence for {}ms", duration);
                let duration_secs = duration as f32 / 1000.0;
                let buf = AudioBuffer::silence(duration_secs);
                // Silence is part of the speech stream
                if let Err(e) = streams.queue(StreamType::Speech, &buf) {
                    warn!("Silence queue error: {}", e);
                }
            }

            QueueItem::AudioIcon { path } => {
                debug!("Playing audio icon: {}", path.display());
                match loader.load(&path) {
                    Ok(mut buf) => {
                        let pipeline = build_sound_pipeline(state);
                        if let Err(e) = pipeline.process(&mut buf) {
                            warn!("Pipeline error: {}", e);
                        }
                        if let Err(e) = streams.queue(StreamType::Sound, &buf) {
                            warn!("Sound queue error: {}", e);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to load audio icon {}: {}", path.display(), e);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Insert space before uppercase letters (for split caps)
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

/// Extract voice from codes like `[{voice en-US:Samantha}]`
fn extract_voice(codes: &str) -> Option<String> {
    let re = regex::Regex::new(r"\[\{voice\s+([^\}]+)\}\]").ok()?;
    re.captures(codes)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

/// Extract pitch from codes like `[[pitch 1.5]]`
fn extract_pitch(codes: &str) -> Option<String> {
    let re = regex::Regex::new(r"\[\[pitch\s+([^\]]+)\]\]").ok()?;
    re.captures(codes)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

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
        let codes = "[{voice en-US:Samantha}]";
        assert_eq!(extract_voice(codes), Some("en-US:Samantha".to_string()));

        let codes2 = "[{voice en-GB:Daniel}]";
        assert_eq!(extract_voice(codes2), Some("en-GB:Daniel".to_string()));

        assert_eq!(extract_voice("no voice here"), None);
    }

    #[test]
    fn test_extract_pitch() {
        let codes = "[[pitch 1.5]]";
        assert_eq!(extract_pitch(codes), Some("1.5".to_string()));

        let codes2 = "[[pitch 0.8]]";
        assert_eq!(extract_pitch(codes2), Some("0.8".to_string()));

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
    fn test_build_speech_pipeline() {
        let state = TtsState::default();
        let pipeline = build_speech_pipeline(&state);
        assert_eq!(pipeline.len(), 3); // trimmer + volume + router
    }

    #[test]
    fn test_build_tone_pipeline() {
        let state = TtsState::default();
        let pipeline = build_tone_pipeline(&state);
        assert_eq!(pipeline.len(), 2); // volume + router
    }

    #[test]
    fn test_build_sound_pipeline() {
        let state = TtsState::default();
        let pipeline = build_sound_pipeline(&state);
        assert_eq!(pipeline.len(), 2); // volume + router
    }

    #[test]
    fn test_apply_punctuation_none() {
        assert_eq!(apply_punctuation("price is $5", PunctuationLevel::None), "price is  dollar 5");
        assert_eq!(apply_punctuation("100%", PunctuationLevel::None), "100 percent ");
        // Other punctuation passes through
        assert_eq!(apply_punctuation("hello!", PunctuationLevel::None), "hello!");
        assert_eq!(apply_punctuation("a@b.com", PunctuationLevel::None), "a@b.com");
    }

    #[test]
    fn test_apply_punctuation_some() {
        assert_eq!(apply_punctuation("$5", PunctuationLevel::Some), " dollar 5");
        assert_eq!(apply_punctuation("a+b=c", PunctuationLevel::Some), "a plus b equals c");
        assert_eq!(apply_punctuation("(hi)", PunctuationLevel::Some), " left paren hi right paren ");
        assert_eq!(apply_punctuation("a/b", PunctuationLevel::Some), "a slash b");
        // @, _, . not spoken in Some mode
        assert_eq!(apply_punctuation("a@b.c", PunctuationLevel::Some), "a@b.c");
    }

    #[test]
    fn test_apply_punctuation_all() {
        assert_eq!(apply_punctuation("a@b", PunctuationLevel::All), "a at b");
        assert_eq!(apply_punctuation("a.b", PunctuationLevel::All), "a dot b");
        assert_eq!(apply_punctuation("a,b", PunctuationLevel::All), "a comma b");
        assert_eq!(apply_punctuation("a_b", PunctuationLevel::All), "a underline b");
        assert_eq!(apply_punctuation("a'b", PunctuationLevel::All), "a apostrophe b");
        assert_eq!(apply_punctuation("a[0]", PunctuationLevel::All), "a left bracket 0 right bracket ");
        assert_eq!(apply_punctuation("a{b}", PunctuationLevel::All), "a left brace b right brace ");
        assert_eq!(apply_punctuation("why?", PunctuationLevel::All), "why question ");
        assert_eq!(apply_punctuation("a|b", PunctuationLevel::All), "a pipe b");
        assert_eq!(apply_punctuation("a&b", PunctuationLevel::All), "a ampersand b");
    }

    #[test]
    fn test_apply_punctuation_plain_text() {
        // Plain text should pass through unchanged at all levels
        assert_eq!(apply_punctuation("hello world", PunctuationLevel::None), "hello world");
        assert_eq!(apply_punctuation("hello world", PunctuationLevel::Some), "hello world");
        assert_eq!(apply_punctuation("hello world", PunctuationLevel::All), "hello world");
    }

    #[test]
    fn test_expand_tilde() {
        // Should return path as-is when no tilde
        assert_eq!(expand_tilde("/foo/bar"), std::path::PathBuf::from("/foo/bar"));
        assert_eq!(expand_tilde("relative/path"), std::path::PathBuf::from("relative/path"));
    }

    #[test]
    fn test_home_dir() {
        // home_dir should return something on all platforms we support
        assert!(home_dir().is_some());
    }

    #[test]
    fn test_parse_args_defaults() {
        // Can't easily test parse_args since it reads std::env::args,
        // but we can test the engine name logic
        assert!(!native_engine_name().is_empty());
    }

    #[test]
    fn test_preprocess_text() {
        let mut state = TtsState::default();
        state.punctuation_level = PunctuationLevel::Some;
        state.split_caps = true;
        assert_eq!(preprocess_text("helloWorld+1", &state), "hello World plus 1");
    }
}
