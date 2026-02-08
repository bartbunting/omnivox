//! Omnivox CLI - Emacspeak Speech Server
//!
//! Cross-platform text-to-speech server implementing the Emacspeak protocol.
//! Uses a buffer-based audio pipeline: TTS/tone/file -> pipeline -> output.

use anyhow::Result;
use omnivox_audio::{
    AudioBuffer, AudioFileLoader, AudioOutput, AudioPipeline, ChannelRouter, SilenceTrimmer,
    ToneGenerator, VolumeAdjust,
};
use omnivox_core::{
    parse_command,
    state::PunctuationLevel,
    Command, CommandId, CommandQueue, QueueItem, TtsState,
};
use omnivox_tts::espeak::EspeakTtsEngine;
#[cfg(target_os = "macos")]
use omnivox_tts::macos::MacOsTtsEngine;
use omnivox_tts::{TtsEngine, TtsSettings};
use std::io::{self, BufRead};
use std::path::Path;
use tracing::{debug, error, info, warn};

const VERSION: &str = env!("CARGO_PKG_VERSION");

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

/// Process text before synthesis: apply punctuation and split caps.
fn preprocess_text(text: &str, state: &TtsState) -> String {
    let mut processed = apply_punctuation(text, state.punctuation_level);
    if state.split_caps {
        processed = insert_space_before_uppercase(&processed);
    }
    processed
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
/// Set OMNIVOX_ENGINE=espeak to force espeak-ng, or OMNIVOX_ENGINE=native for platform default.
fn create_engine() -> Result<Box<dyn TtsEngine>> {
    let forced = std::env::var("OMNIVOX_ENGINE").unwrap_or_default();

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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    info!("Omnivox v{} starting", VERSION);

    let engine = create_engine()?;
    info!("TTS engine initialized");

    let voices = engine.available_voices();
    info!("Found {} voices", voices.len());

    let output = AudioOutput::new().map_err(|e| anyhow::anyhow!("Audio output init failed: {}", e))?;
    let loader = AudioFileLoader::with_cache();

    let mut state = TtsState::default();
    let mut queue = CommandQueue::new();
    let mut current_playback: Option<omnivox_audio::output::PlaybackHandle> = None;

    // Speak version
    let settings = TtsSettings::default();
    let version_text = format!("Omnivox version {}", VERSION.replace('.', " dot "));
    if let Ok(tts_buf) = engine.synthesize(&version_text, &settings) {
        let mut buf = tts_buffer_to_audio_buffer(tts_buf);
        let pipeline = build_speech_pipeline(&state);
        let _ = pipeline.process(&mut buf);
        if let Ok(handle) = output.play(&buf) {
            handle.wait();
        }
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
                    &output,
                    &loader,
                    &mut current_playback,
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
    output: &AudioOutput,
    loader: &AudioFileLoader,
    current_playback: &mut Option<omnivox_audio::output::PlaybackHandle>,
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
                debug!("Queuing audio icon: {}", path);
                queue.enqueue(QueueItem::AudioIcon {
                    path: path.into(),
                });
            }
        }

        // Dispatch queue
        CommandId::Dispatch => {
            debug!("Dispatching queue ({} items)", queue.len());
            let items = queue.dispatch();
            process_queue_items(items, state, engine, output, loader, current_playback).await?;
        }

        // Immediate commands
        CommandId::Stop => {
            debug!("Stopping speech");
            if let Some(handle) = current_playback.take() {
                handle.stop();
            }
            engine.stop();
            queue.clear();
        }

        CommandId::Letter => {
            if let Some(letter) = command.args {
                debug!("Speaking letter: {}", letter);
                let old_rate = state.speech_rate;
                let old_pitch = state.pitch_multiplier;

                state.speech_rate = state.character_rate();

                if letter.chars().next().map_or(false, |c| c.is_uppercase()) {
                    if state.allcaps_beep {
                        // Play a short beep for capital letters
                        let tone_buf = ToneGenerator::generate(440.0, 10, state.tone_volume);
                        let mut buf = tone_buf;
                        let pipeline = build_tone_pipeline(state);
                        let _ = pipeline.process(&mut buf);
                        if let Ok(handle) = output.play(&buf) {
                            handle.wait();
                        }
                    } else {
                        state.pitch_multiplier = 1.5;
                    }
                }

                let settings = TtsSettings {
                    voice: state.current_voice.clone(),
                    rate: state.speech_rate,
                    pitch: state.pitch_multiplier,
                    volume: state.voice_volume,
                };

                if let Ok(tts_buf) = engine.synthesize(&letter.to_lowercase(), &settings) {
                    let mut buf = tts_buffer_to_audio_buffer(tts_buf);
                    let pipeline = build_speech_pipeline(state);
                    let _ = pipeline.process(&mut buf);
                    if let Ok(handle) = output.play(&buf) {
                        handle.wait();
                    }
                }

                state.speech_rate = old_rate;
                state.pitch_multiplier = old_pitch;
            }
        }

        CommandId::TtsSay => {
            if let Some(text) = command.args {
                let processed_text = preprocess_text(&text, state);
                debug!("Speaking immediately: {}", processed_text);
                if let Some(handle) = current_playback.take() {
                    handle.stop();
                }
                engine.stop();
                let settings = TtsSettings {
                    voice: state.current_voice.clone(),
                    rate: state.speech_rate,
                    pitch: state.pitch_multiplier,
                    volume: state.voice_volume,
                };
                if let Ok(tts_buf) = engine.synthesize(&processed_text, &settings) {
                    let mut buf = tts_buffer_to_audio_buffer(tts_buf);
                    let pipeline = build_speech_pipeline(state);
                    let _ = pipeline.process(&mut buf);
                    if let Ok(handle) = output.play(&buf) {
                        handle.wait();
                    }
                }
            }
        }

        CommandId::PlaySound => {
            if let Some(path) = command.args {
                debug!("Playing sound immediately: {}", path);
                if let Some(handle) = current_playback.take() {
                    handle.stop();
                }
                match loader.load(Path::new(&path)) {
                    Ok(mut buf) => {
                        let pipeline = build_sound_pipeline(state);
                        let _ = pipeline.process(&mut buf);
                        if let Ok(handle) = output.play(&buf) {
                            handle.wait();
                        }
                    }
                    Err(e) => {
                        warn!("Failed to load audio file {}: {}", path, e);
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
                if let Ok(handle) = output.play(&buf) {
                    handle.wait();
                }
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
            if let Some(handle) = current_playback.take() {
                handle.stop();
            }
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

/// Process queue items after dispatch
async fn process_queue_items(
    items: Vec<QueueItem>,
    state: &mut TtsState,
    engine: &dyn TtsEngine,
    output: &AudioOutput,
    loader: &AudioFileLoader,
    current_playback: &mut Option<omnivox_audio::output::PlaybackHandle>,
) -> Result<()> {
    for item in items {
        match item {
            QueueItem::Speech(text) => {
                let settings = TtsSettings {
                    voice: state.current_voice.clone(),
                    rate: state.speech_rate,
                    pitch: state.pitch_multiplier,
                    volume: state.voice_volume,
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
                        match output.play(&buf) {
                            Ok(handle) => {
                                handle.wait();
                                *current_playback = None;
                            }
                            Err(e) => warn!("Playback error: {}", e),
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
                match output.play(&buf) {
                    Ok(handle) => {
                        handle.wait();
                    }
                    Err(e) => warn!("Playback error: {}", e),
                }
            }

            QueueItem::Silence { duration } => {
                debug!("Silence for {}ms", duration);
                let duration_secs = duration as f32 / 1000.0;
                let buf = AudioBuffer::silence(duration_secs);
                match output.play(&buf) {
                    Ok(handle) => {
                        handle.wait();
                    }
                    Err(e) => warn!("Playback error: {}", e),
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
                        match output.play(&buf) {
                            Ok(handle) => {
                                handle.wait();
                            }
                            Err(e) => warn!("Playback error: {}", e),
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
    let mut prev_was_lower = false;

    for c in input.chars() {
        if c.is_uppercase() {
            if !result.is_empty() && (prev_was_lower || !prev_was_lower) {
                result.push(' ');
            }
            prev_was_lower = false;
        } else {
            prev_was_lower = c.is_lowercase();
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
    fn test_preprocess_text() {
        let mut state = TtsState::default();
        state.punctuation_level = PunctuationLevel::Some;
        state.split_caps = true;
        assert_eq!(preprocess_text("helloWorld+1", &state), "hello World plus 1");
    }
}
