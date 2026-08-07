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
mod health;
mod marker_events;
mod pipeline;
mod routing;
mod server;
mod text;
mod transaction;

use anyhow::Result;
use omnivox_audio::AudioFileLoader;
use omnivox_audio::AudioStreams;
use omnivox_core::TtsState;
use std::any::Any;
use std::backtrace::Backtrace;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::AtomicU64;
use std::sync::{mpsc, Arc};
use tracing::{error, info};

use cli::{apply_cli_flags, parse_args};
use engine::{apply_audio_target_env, create_engine, create_engines};
use health::RuntimeEngineHealth;
use marker_events::spawn_marker_event_reporter;
use server::{
    run_server, spawn_tracked_playback_reporter, synthesis_worker, SynthRequest,
};

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

const SYNTHESIS_WORKER_FAILURE_EXIT_CODE: i32 = 70;

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
    install_panic_diagnostics();

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
    let runtime_health = Arc::new(RuntimeEngineHealth::new());
    let (marker_output, marker_event_handle) = spawn_marker_event_reporter();
    let (tracked_playback_tx, tracked_playback_handle) =
        spawn_tracked_playback_reporter(marker_output.clone());

    let worker_handle = {
        let worker_engine = engine.clone();
        let worker_engine_registry = engine_registry.clone();
        let worker_runtime_health = runtime_health.clone();
        let worker_control = control.clone();
        let worker_gen = gen_counter.clone();
        let loader = AudioFileLoader::with_cache();
        std::thread::Builder::new()
            .name("omnivox-synth".to_string())
            .spawn(move || {
                let result = panic::catch_unwind(AssertUnwindSafe(|| {
                    synthesis_worker(
                        rx,
                        worker_gen,
                        worker_engine,
                        worker_engine_registry,
                        worker_runtime_health,
                        worker_control,
                        loader,
                        tracked_playback_tx,
                        marker_output,
                    )
                }));
                if let Err(payload) = result {
                    error!(
                        panic = panic_payload_message(&*payload),
                        exit_code = SYNTHESIS_WORKER_FAILURE_EXIT_CODE,
                        "Synthesis worker panicked; terminating the speech server"
                    );
                    std::process::exit(SYNTHESIS_WORKER_FAILURE_EXIT_CODE);
                }
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
                    runtime_health,
                    state,
                    tx,
                    control,
                    gen_counter,
                    worker_handle,
                    tracked_playback_handle,
                    marker_event_handle,
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
        runtime_health,
        state,
        tx,
        control,
        gen_counter,
        worker_handle,
        tracked_playback_handle,
        marker_event_handle,
    )
}

fn install_panic_diagnostics() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |information| {
        let location = information
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown".to_owned());
        error!(
            panic = panic_payload_message(information.payload()),
            %location,
            backtrace = %Backtrace::force_capture(),
            "Omnivox panic"
        );
        previous(information);
    }));
}

fn panic_payload_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}
