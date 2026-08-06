//! Omnivox CLI - Emacspeak Speech Server
//!
//! Cross-platform text-to-speech server implementing the Emacspeak protocol.
//! Uses a buffer-based audio pipeline: TTS/tone/file -> pipeline -> output.
//!
//! # Threading Model
//!
//! - **Reader thread** (main): reads stdin and parses commands in a tight loop.
//!   Never blocks on synthesis. Stop/reset commands take effect immediately.
//!
//! - **Synthesis worker** (spawned): receives `SynthRequest`s via an unbounded
//!   channel, synthesizes each chunk, checks the generation counter between
//!   chunks, and queues audio to rodio. Stale requests are discarded.
//!
//! Audio is played on three concurrent streams (speech, tones, sounds).
//! Items within each stream serialize; different streams overlap.

mod cli;
mod engine;
mod pipeline;
mod routing;
mod server;
mod text;
mod transaction;

use anyhow::Result;
use omnivox_audio::AudioFileLoader;
use omnivox_audio::AudioStreams;
use omnivox_core::TtsState;
use std::sync::atomic::AtomicU64;
use std::sync::{mpsc, Arc};
use tracing::info;

use cli::{apply_cli_flags, parse_args};
use engine::{apply_audio_target_env, create_engine, create_engines};
use server::{run_server, synthesis_worker, SynthRequest};

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) const SPEECH_MAX_DEPTH: usize = 100;
pub(crate) const TONE_MAX_DEPTH: usize = 10;
pub(crate) const SOUND_MAX_DEPTH: usize = 10;

fn main() -> Result<()> {
    let cli = parse_args();

    match cli.action.as_str() {
        "help" => { cli::print_help(); return Ok(()); }
        "version" => { cli::print_version(); return Ok(()); }
        "check" => { cli::cmd_check(&cli.engine); return Ok(()); }
        "list-voices" => {
            let engine = create_engine(&cli.engine, cli.piper_model.as_deref())?;
            cli::cmd_list_voices(engine.as_ref());
            return Ok(());
        }
        "list-voices-alist" => {
            let engine = create_engine(&cli.engine, cli.piper_model.as_deref())?;
            cli::cmd_list_voices_alist(engine.as_ref());
            return Ok(());
        }
        "play-wav" => {
            let remaining: Vec<String> = std::env::args().collect();
            let idx = remaining.iter().position(|a| a == "--play-wav").unwrap_or(0);
            if idx + 1 >= remaining.len() {
                eprintln!("Usage: omnivox --play-wav <file.wav>");
                std::process::exit(1);
            }
            cli::cmd_play_wav(&remaining[idx + 1]);
            return Ok(());
        }
        "dump-wav" => {
            let remaining: Vec<String> = std::env::args().collect();
            let dump_idx = remaining.iter().position(|a| a == "--dump-wav").unwrap_or(0);
            let dump_args: Vec<&str> = remaining[dump_idx + 1..].iter().map(|s| s.as_str()).collect();
            if dump_args.len() < 2 {
                eprintln!("Usage: omnivox --dump-wav <voice> <output.wav> [text...]");
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
            cli::cmd_dump_wav(&cli.engine, voice, output, &text);
            return Ok(());
        }
        _ => {}
    }

    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    info!("Omnivox v{} starting", VERSION);

    let created_engines = {
        #[cfg(target_os = "macos")]
        { info!("Initializing macOS TTS engine (ObjC bridge)"); }
        create_engines(&cli.engine, cli.piper_model.as_deref())?
    };
    let engine = created_engines.preferred;
    let engine_registry = Arc::new(created_engines.registry);
    info!("TTS engines initialized");
    let voice_count = engine_registry
        .inventory()
        .iter()
        .map(|descriptor| descriptor.voices.len())
        .sum::<usize>();
    info!("Found {} voices", voice_count);

    let streams = AudioStreams::new(SPEECH_MAX_DEPTH, TONE_MAX_DEPTH, SOUND_MAX_DEPTH)
        .map_err(|e| anyhow::anyhow!("Audio streams init failed: {}", e))?;
    let control = streams.control();

    let mut state = TtsState::default();
    apply_audio_target_env(&mut state);
    apply_cli_flags(&cli, &mut state);

    let (tx, rx) = mpsc::channel::<SynthRequest>();
    let gen_counter = Arc::new(AtomicU64::new(0));

    let worker_handle = {
        let worker_engine = engine.clone();
        let worker_engine_registry = engine_registry.clone();
        let worker_control = control.clone();
        let worker_gen = gen_counter.clone();
        let loader = AudioFileLoader::with_cache();
        std::thread::Builder::new()
            .name("omnivox-synth".to_string())
            .spawn(move || {
                synthesis_worker(
                    rx,
                    worker_gen,
                    worker_engine,
                    worker_engine_registry,
                    worker_control,
                    loader,
                )
            })
            .expect("Failed to spawn synthesis worker thread")
    };

    // On macOS, AVSpeechSynthesizer.writeUtterance:toBufferCallback: internally
    // uses the main GCD queue. If the main thread is blocked on stdin instead of
    // running a NSRunLoop, synthesis deadlocks. Fix: run the reader on a background
    // thread; main thread keeps AudioStreams alive and pumps the NSRunLoop.
    #[cfg(target_os = "macos")]
    {
        use std::sync::Mutex;
        let result: Arc<Mutex<Option<Result<()>>>> = Arc::new(Mutex::new(None));
        let result2 = result.clone();
        std::thread::Builder::new()
            .name("omnivox-reader".to_string())
            .spawn(move || {
                let r = run_server(
                    engine,
                    engine_registry,
                    state,
                    tx,
                    control,
                    gen_counter,
                    worker_handle,
                );
                *result2.lock().unwrap() = Some(r);
                omnivox_tts::macos::stop_main_runloop();
            })
            .expect("Failed to spawn reader thread");
        omnivox_tts::macos::run_main_runloop();
        drop(streams);
        return result.lock().unwrap().take().unwrap_or(Ok(()));
    }

    #[cfg(not(target_os = "macos"))]
    run_server(
        engine,
        engine_registry,
        state,
        tx,
        control,
        gen_counter,
        worker_handle,
    )
}
