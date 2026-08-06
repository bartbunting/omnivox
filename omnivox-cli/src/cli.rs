//! CLI argument parsing and non-server commands (--check, --list-voices, etc.).

use anyhow::Result;
use omnivox_audio::{AudioFileLoader, AudioStreams, StreamType, ToneGenerator};
use omnivox_core::state::ChannelMode;
use omnivox_core::TtsState;
use omnivox_tts::{TtsEngine, TtsSettings};
use std::io::Write as IoWrite;
use std::sync::Arc;
use tracing::{info, warn};

use crate::engine::{create_engine, native_engine_name};
use crate::pipeline::tts_buffer_to_audio_buffer;
use crate::text::home_dir;

// ---------------------------------------------------------------------------
// CLI arguments
// ---------------------------------------------------------------------------

pub struct CliArgs {
    pub engine: String,
    pub action: String,
    pub voice: Option<String>,
    pub rate: Option<f32>,
    pub pitch: Option<f32>,
    pub voice_volume: Option<f32>,
    pub tone_volume: Option<f32>,
    pub sound_volume: Option<f32>,
    pub audio_target: Option<String>,
    /// Path to a piper `.onnx` model file (overrides `OMNIVOX_PIPER_MODEL`).
    pub piper_model: Option<String>,
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

pub fn parse_args() -> CliArgs {
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
        piper_model: None,
    };

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h"          => cli.action = String::from("help"),
            "--version" | "-V"       => cli.action = String::from("version"),
            "--check"                => cli.action = String::from("check"),
            "--list-voices"          => cli.action = String::from("list-voices"),
            "--list-voices-alist"    => cli.action = String::from("list-voices-alist"),
            "--dump-wav"             => cli.action = String::from("dump-wav"),
            "--play-wav"             => cli.action = String::from("play-wav"),
            "--engine"       => cli.engine        = parse_string_flag("--engine",        &args, &mut i),
            "--voice"        => cli.voice         = Some(parse_string_flag("--voice",        &args, &mut i)),
            "--rate"         => cli.rate          = Some(parse_float_flag("--rate",          &args, &mut i)),
            "--pitch"        => cli.pitch         = Some(parse_float_flag("--pitch",         &args, &mut i)),
            "--voice-volume" => cli.voice_volume  = Some(parse_float_flag("--voice-volume",  &args, &mut i)),
            "--tone-volume"  => cli.tone_volume   = Some(parse_float_flag("--tone-volume",   &args, &mut i)),
            "--sound-volume" => cli.sound_volume  = Some(parse_float_flag("--sound-volume",  &args, &mut i)),
            "--audio-target" => cli.audio_target  = Some(parse_string_flag("--audio-target", &args, &mut i)),
            "--piper-model"  => cli.piper_model   = Some(parse_string_flag("--piper-model",  &args, &mut i)),
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

pub fn apply_cli_flags(cli: &CliArgs, state: &mut TtsState) {
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
// Informational commands
// ---------------------------------------------------------------------------

pub fn print_help() {
    let native = native_engine_name();
    println!("Omnivox v{} - Cross-platform Emacspeak speech server", crate::VERSION);
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
    println!("    --engine NAME    Select TTS engine: native, espeak, piper");
    println!("    --voice ID       Set default voice (e.g. en-US:Alex)");
    println!("    --rate FLOAT     Speech rate 0.0-1.0 (0.5 = normal)");
    println!("    --pitch FLOAT    Pitch multiplier 0.5-2.0 (1.0 = normal)");
    println!("    --voice-volume F Voice volume 0.0-1.0");
    println!("    --tone-volume F  Tone volume 0.0-1.0");
    println!("    --sound-volume F Sound/icon volume 0.0-1.0");
    println!("    --audio-target T Channel routing (left, right, both)");
    println!("    --piper-model P  Path to a piper .onnx model file (use with --engine piper)");
    println!("    --dump-wav VOICE OUTPUT [TEXT]");
    println!("                     Save TTS output to WAV files for analysis");
    println!("    --play-wav FILE  Play a WAV file through the rodio audio path");
    println!();
    println!("ENGINES:");
    println!("    native    Platform-native TTS: {}", native);
    println!("    espeak    espeak-ng (cross-platform, always available)");
    println!("    piper     Piper neural TTS (build with --features piper; requires --piper-model)");
    println!();
    println!("Without options, starts the Emacspeak protocol server on stdin.");
    println!();
    println!("ENVIRONMENT (for Emacspeak integration only):");
    println!("    OMNIVOX_ENGINE         Same as --engine");
    println!("    OMNIVOX_PIPER_MODEL    Path to piper .onnx model (same as --piper-model)");
    println!("    OMNIVOX_AUDIO_TARGET   Same as --audio-target (set by Emacspeak notification mode)");
    println!();
    println!("EMACSPEAK SETUP:");
    println!("    (setq dtk-program \"omnivox\")");
    println!("    Ensure omnivox is in your PATH or in emacspeak/servers/");
}

pub fn print_version() {
    println!("omnivox {}", crate::VERSION);
}

pub fn cmd_list_voices(engine: &dyn TtsEngine) {
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

fn escape_elisp_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn format_voices_alist(voices: &[omnivox_tts::VoiceInfo]) -> String {
    let mut output = String::from("(");
    for (index, voice) in voices.iter().enumerate() {
        if index > 0 {
            output.push_str("\n ");
        }
        output.push_str(&format!(
            "(\"{}\" \"{}\" \"{}\" \"{:?}\")",
            escape_elisp_string(&voice.identifier),
            escape_elisp_string(&voice.name),
            escape_elisp_string(&voice.language),
            voice.quality,
        ));
    }
    output.push(')');
    output
}

pub fn cmd_list_voices_alist(engine: &dyn TtsEngine) {
    let voices = engine.available_voices();
    println!("{}", format_voices_alist(&voices));
}

pub fn cmd_check(engine_name: &str) {
    println!("Omnivox v{} diagnostic check", crate::VERSION);
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
    let engine: Arc<dyn TtsEngine> = match create_engine(engine_name, None) {
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
                    buf.samples.len(), buf.sample_rate, buf.channels
                );
            }
        }
        Err(e) => println!("  Status: FAILED - {}", e),
    }
    println!();

    println!("[audio output]");
    match AudioStreams::new(crate::SPEECH_MAX_DEPTH, crate::TONE_MAX_DEPTH, crate::SOUND_MAX_DEPTH) {
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
    let test_paths = ["test-sounds/button.ogg", "test-sounds/complete.ogg"];
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

// ---------------------------------------------------------------------------
// WAV debug commands
// ---------------------------------------------------------------------------

const WAV_FMT_CHUNK_SIZE: u32 = 16;
const WAV_FORMAT_IEEE_FLOAT: u16 = 3;
const WAV_BITS_PER_SAMPLE: u16 = 32;

pub fn write_wav(path: &str, samples: &[f32], sample_rate: u32, channels: u16) -> Result<()> {
    let mut f = std::fs::File::create(path)?;

    let num_samples = samples.len() as u32;
    let byte_rate = sample_rate * channels as u32 * (WAV_BITS_PER_SAMPLE as u32 / 8);
    let block_align = channels * (WAV_BITS_PER_SAMPLE / 8);
    let data_size = num_samples * (WAV_BITS_PER_SAMPLE as u32 / 8);
    let file_size = 36 + data_size;

    f.write_all(b"RIFF")?;
    f.write_all(&file_size.to_le_bytes())?;
    f.write_all(b"WAVE")?;
    f.write_all(b"fmt ")?;
    f.write_all(&WAV_FMT_CHUNK_SIZE.to_le_bytes())?;
    f.write_all(&WAV_FORMAT_IEEE_FLOAT.to_le_bytes())?;
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&block_align.to_le_bytes())?;
    f.write_all(&WAV_BITS_PER_SAMPLE.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_size.to_le_bytes())?;
    for &s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}

pub fn cmd_dump_wav(engine_name: &str, voice: &str, output: &str, text: &str) {
    use crate::pipeline::{build_speech_pipeline, tts_buffer_to_audio_buffer};
    use omnivox_audio::AudioBuffer;

    let engine = match create_engine(engine_name, None) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to create engine: {}", e);
            std::process::exit(1);
        }
    };

    let mut state = TtsState::default();
    if !voice.is_empty() {
        let parts: Vec<&str> = voice.splitn(2, ':').collect();
        let _ = parts; // splitn result unused; voice is always valid either way
        state.current_voice = voice.to_string();
    }

    let settings = TtsSettings {
        voice: state.current_voice.clone(),
        rate: state.speech_rate,
        pitch: state.pitch_multiplier,
        volume: 1.0,
    };

    match engine.synthesize(text, &settings) {
        Ok(tts_buf) => {
            let raw_path = output.replace(".wav", "_raw.wav");
            match write_wav(&raw_path, &tts_buf.samples, tts_buf.sample_rate, tts_buf.channels) {
                Ok(_) => println!("Raw: {} ({} samples, {}Hz, {}ch)", raw_path, tts_buf.samples.len(), tts_buf.sample_rate, tts_buf.channels),
                Err(e) => eprintln!("Failed to write {}: {}", raw_path, e),
            }

            let mut buf: AudioBuffer = tts_buffer_to_audio_buffer(tts_buf);
            let pipeline = build_speech_pipeline(&state, true);
            if let Err(e) = pipeline.process(&mut buf) {
                eprintln!("Pipeline error: {}", e);
            }
            match write_wav(output, &buf.samples, 44100, 2) {
                Ok(_) => println!("Pipeline: {} ({} samples, 44100Hz, 2ch)", output, buf.samples.len()),
                Err(e) => eprintln!("Failed to write {}: {}", output, e),
            }
        }
        Err(e) => {
            eprintln!("Synthesis failed: {}", e);
            std::process::exit(1);
        }
    }
}

pub fn cmd_play_wav(path: &str) {
    let streams = match AudioStreams::new(crate::SPEECH_MAX_DEPTH, crate::TONE_MAX_DEPTH, crate::SOUND_MAX_DEPTH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Audio init failed: {}", e);
            std::process::exit(1);
        }
    };

    let loader = AudioFileLoader::with_cache();
    match loader.load(std::path::Path::new(path)) {
        Ok(buf) => {
            println!("Playing {} ({} samples)...", path, buf.samples.len());
            match streams.queue(StreamType::Speech, &buf) {
                Ok(_) => {}
                Err(e) => eprintln!("Queue failed: {}", e),
            }
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
        Err(e) => {
            eprintln!("Failed to load {}: {}", path, e);
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_tts::{VoiceInfo, VoiceQuality};

    #[test]
    fn test_format_voices_alist_preserves_backend_identifier() {
        let voices = vec![VoiceInfo {
            identifier: "winrt:HKEY\\Voice\"One".to_string(),
            name: "Voice \"One\"".to_string(),
            language: "en-US".to_string(),
            quality: VoiceQuality::Enhanced,
        }];

        assert_eq!(
            format_voices_alist(&voices),
            r#"(("winrt:HKEY\\Voice\"One" "Voice \"One\"" "en-US" "Enhanced"))"#
        );
    }

    #[test]
    fn test_escape_elisp_string_handles_protocol_unsafe_characters() {
        assert_eq!(
            escape_elisp_string("line\ncarriage\rtab\tcontrol\u{1f}"),
            "line\\ncarriage\\rtab\\tcontrol\\u001f"
        );
    }
}
