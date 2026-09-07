// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later

use super::markers::{Mark, MAX_MARKERS};
use libloading::Library;
use omnivox_audio::ProgressivePcmCanonicalizer;
use omnivox_tts::contracts::*;
use omnivox_tts::helper_protocol::MAX_HELPER_SYNTHESIS_BYTES;
use omnivox_tts::*;
use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(super) const SAMPLE_RATE: u32 = 11025;
const QUEUE_CAPACITY: usize = 4;

// Match the Windows adapters' established ten-level ACSS interpolation.
pub(super) fn map_level(value: f32, levels: &[i32; 10]) -> i32 {
    let position = f64::from(value.clamp(0.0, 1.0)) * 9.0;
    let lower = position.floor() as usize;
    let upper = (lower + 1).min(9);
    (f64::from(levels[lower])
        + f64::from(levels[upper] - levels[lower]) * (position - lower as f64))
        .round() as i32
}

pub(super) trait Runtime {
    fn descriptor(&self) -> EngineDescriptor;
    fn synthesize(&mut self, request: &SynthesisRequest, capture: Capture) -> Result<(), String>;
}

pub(super) struct Engine {
    descriptor: EngineDescriptor,
    jobs: Option<SyncSender<(SynthesisRequest, Capture)>>,
    stop_epoch: Arc<AtomicU64>,
    synthesis: Mutex<()>,
    speaking: AtomicBool,
}

impl Engine {
    pub(super) fn new(id: &'static str) -> Self {
        let stop_epoch = Arc::new(AtomicU64::new(0));
        let (jobs, receiver) = sync_channel::<(SynthesisRequest, Capture)>(1);
        let (ready, initialized) = sync_channel(1);
        // Keep creation, synthesis and destruction on one native owner thread.
        // ECI callbacks and global legacy APIs must not migrate between jobs.
        let spawned = std::thread::Builder::new()
            .name(format!("{id}-owner"))
            .spawn(move || {
                let runtime: Result<Box<dyn Runtime>, String> = match id {
                    "eloquence" => super::eloquence::load(),
                    "dectalk" => super::dectalk::load(),
                    _ => Err("unknown Linux helper".to_owned()),
                };
                match runtime {
                    Ok(mut runtime) => {
                        if ready.send(Ok(runtime.descriptor())).is_err() {
                            return;
                        }
                        while let Ok((request, capture)) = receiver.recv() {
                            let sender = capture.sender.clone();
                            let result = runtime.synthesize(&request, capture);
                            if sender.send(Event::Finished(result)).is_err() {
                                // The client can abandon a cancelled request; a
                                // subsequent job still owns a fresh capture.
                                continue;
                            }
                        }
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                    }
                }
            });
        let result = spawned.map_err(|e| e.to_string()).and_then(|_| {
            initialized
                .recv_timeout(Duration::from_secs(10))
                .map_err(|e| format!("runtime initialization: {e}"))?
        });
        let (descriptor, jobs) = match result {
            Ok(descriptor) => (descriptor, Some(jobs)),
            Err(error) => (EngineDescriptor::unavailable(id, error), None),
        };
        Self {
            descriptor,
            jobs,
            stop_epoch,
            synthesis: Mutex::new(()),
            speaking: AtomicBool::new(false),
        }
    }
}

impl TtsEngine for Engine {
    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor.clone()
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let mut collector = Collector::default();
        self.synthesize_stream(request, &mut collector)?;
        let start = collector.start.expect("successful stream has metadata");
        let mut result = SynthesisResult::audio(
            start.engine_id,
            start.actual_voice,
            AudioBuffer::new(collector.samples),
        );
        result.degraded_acss = start.degraded_acss;
        result.markers = collector.markers;
        result.anchors = collector.anchors;
        Ok(result)
    }

    fn synthesize_stream(
        &self,
        request: &SynthesisRequest,
        sink: &mut dyn SynthesisStreamSink,
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        let jobs = self.jobs.as_ref().ok_or(TtsError::NotAvailable)?;
        let _guard = self
            .synthesis
            .lock()
            .map_err(|e| TtsError::SynthesisFailed(e.to_string()))?;
        let voice = request.voice_id_for_engine(&self.descriptor.id)?;
        if !self
            .descriptor
            .voices
            .iter()
            .any(|v| v.id.voice_id == voice)
        {
            return Err(TtsError::VoiceNotFound(voice.to_owned()));
        }
        // Validate encoding before committing stream metadata or entering native code.
        encode_text(&request.text).map_err(TtsError::InvalidParameter)?;
        request.clone().with_anchors(request.anchors.clone())?;
        let (sender, receiver) = sync_channel(QUEUE_CAPACITY);
        let capture = Capture {
            sender,
            cancellation: Cancellation {
                epoch: self.stop_epoch.clone(),
                expected: self.stop_epoch.load(Ordering::Acquire),
                request: request.cancellation.clone(),
                aborted: Arc::new(AtomicBool::new(false)),
            },
            frames: 0,
            marker_count: 0,
        };
        if capture.cancellation.cancelled() {
            return Err(cancelled());
        }
        let aborted = capture.cancellation.aborted.clone();
        sink.start(SynthesisStreamStart {
            engine_id: self.descriptor.id.clone(),
            actual_voice: Some(PhysicalVoiceId::new(&self.descriptor.id, voice)),
            degraded_acss: request
                .normalized_acss
                .clone()
                .degrade_for(&self.descriptor.capabilities.acss)
                .omitted,
        })?;
        jobs.send((request.clone(), capture))
            .map_err(|_| TtsError::NotAvailable)?;
        self.speaking.store(true, Ordering::Release);
        let result = (|| {
            let mut converter = ProgressivePcmCanonicalizer::new(SAMPLE_RATE, 1)
                .map_err(|e| TtsError::SynthesisFailed(e.to_string()))?;
            loop {
                match receiver.recv().map_err(|_| {
                    TtsError::SynthesisFailed("native owner disconnected".to_owned())
                })? {
                    Event::Audio(samples) => {
                        let windows = converter
                            .push_interleaved_i16(&samples)
                            .map_err(|e| TtsError::SynthesisFailed(e.to_string()))?;
                        emit(sink, windows, request.settings.volume)?;
                    }
                    Event::Markers(marks) => {
                        let mut markers = Vec::new();
                        let mut anchors = Vec::new();
                        for mark in marks {
                            let frame = converter
                                .canonical_frame_offset(mark.frame())
                                .map_err(|e| TtsError::SynthesisFailed(e.to_string()))?;
                            if frame < converter.output_frames() {
                                return Err(TtsError::SynthesisFailed(
                                    "native marker arrived after its audio".to_owned(),
                                ));
                            }
                            match mark.at(frame) {
                                Mark::Text(mark) => markers.push(mark),
                                Mark::Anchor(mark) => anchors.push(mark),
                            }
                        }
                        sink.markers(markers, anchors)?;
                    }
                    Event::Finished(result) => {
                        result.map_err(TtsError::SynthesisFailed)?;
                        emit(
                            sink,
                            converter
                                .finish()
                                .map_err(|e| TtsError::SynthesisFailed(e.to_string()))?,
                            request.settings.volume,
                        )?;
                        return Ok(SynthesisStreamCompletion {
                            frame_count: converter.output_frames(),
                        });
                    }
                }
            }
        })();
        if result.is_err() {
            aborted.store(true, Ordering::Release);
        }
        drop(receiver);
        self.speaking.store(false, Ordering::Release);
        result
    }

    fn stop(&self) {
        self.stop_epoch.fetch_add(1, Ordering::AcqRel);
    }
    fn is_speaking(&self) -> bool {
        self.speaking.load(Ordering::Acquire)
    }
    fn available_voices(&self) -> Vec<VoiceInfo> {
        self.descriptor
            .voices
            .iter()
            .map(|v| VoiceInfo {
                identifier: v.id.voice_id.clone(),
                name: v.display_name.clone(),
                language: v.language.clone().unwrap_or_default(),
                quality: v.quality,
            })
            .collect()
    }
    fn voice_info(&self, id: &str) -> Option<VoiceInfo> {
        self.available_voices()
            .into_iter()
            .find(|v| v.identifier == id)
    }
}

fn cancelled() -> TtsError {
    TtsError::SynthesisFailed("native synthesis cancelled".to_owned())
}

fn emit(
    sink: &mut dyn SynthesisStreamSink,
    windows: Vec<AudioBuffer>,
    volume: f32,
) -> Result<(), TtsError> {
    for mut audio in windows {
        for sample in &mut audio.samples {
            *sample *= volume.clamp(0.0, 1.0);
        }
        if !audio.is_empty() {
            sink.audio(audio)?;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Collector {
    start: Option<SynthesisStreamStart>,
    samples: Vec<f32>,
    markers: Vec<SynthesisMarker>,
    anchors: Vec<ResolvedAnchor>,
}
impl SynthesisStreamSink for Collector {
    fn start(&mut self, start: SynthesisStreamStart) -> Result<(), TtsError> {
        self.start = Some(start);
        Ok(())
    }
    fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
        if audio.samples.len() > (MAX_HELPER_SYNTHESIS_BYTES / 4).saturating_sub(self.samples.len())
        {
            return Err(TtsError::SynthesisFailed(
                "PCM exceeds collection limit".to_owned(),
            ));
        }
        self.samples.extend(audio.samples);
        Ok(())
    }
    fn markers(
        &mut self,
        markers: Vec<SynthesisMarker>,
        anchors: Vec<ResolvedAnchor>,
    ) -> Result<(), TtsError> {
        self.markers.extend(markers);
        self.anchors.extend(anchors);
        Ok(())
    }
}

enum Event {
    Audio(Vec<i16>),
    Markers(Vec<Mark>),
    Finished(Result<(), String>),
}

#[derive(Clone)]
pub(super) struct Cancellation {
    epoch: Arc<AtomicU64>,
    expected: u64,
    request: Option<SynthesisCancellationToken>,
    pub(super) aborted: Arc<AtomicBool>,
}
impl Cancellation {
    pub(super) fn cancelled(&self) -> bool {
        self.aborted.load(Ordering::Acquire)
            || self.epoch.load(Ordering::Acquire) != self.expected
            || self
                .request
                .as_ref()
                .is_some_and(SynthesisCancellationToken::is_cancelled)
    }
}

pub(super) struct Capture {
    sender: SyncSender<Event>,
    pub(super) cancellation: Cancellation,
    pub(super) frames: usize,
    marker_count: usize,
}
impl Capture {
    pub(super) fn audio(&mut self, samples: &[i16]) -> Result<(), String> {
        if samples.len() > (MAX_HELPER_SYNTHESIS_BYTES / 2).saturating_sub(self.frames) {
            return Err("native PCM exceeds synthesis limit".to_owned());
        }
        self.frames += samples.len();
        if samples.is_empty() {
            return Ok(());
        }
        self.send(Event::Audio(samples.to_vec()))
    }
    pub(super) fn markers(&mut self, mut marks: Vec<Mark>) -> Result<(), String> {
        if marks.len() > MAX_MARKERS.saturating_sub(self.marker_count) {
            return Err("native markers exceed synthesis limit".to_owned());
        }
        if marks.is_empty() {
            return Ok(());
        }
        marks.sort_by_key(Mark::frame);
        if marks[0].frame() < self.frames as u64 {
            return Err("native marker arrived after its audio".to_owned());
        }
        self.marker_count += marks.len();
        self.send(Event::Markers(marks))
    }
    fn send(&self, mut event: Event) -> Result<(), String> {
        loop {
            if self.cancellation.cancelled() {
                return Err("native synthesis cancelled".to_owned());
            }
            match self.sender.try_send(event) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Disconnected(_)) => {
                    return Err("synthesis consumer disconnected".to_owned())
                }
                Err(TrySendError::Full(pending)) => {
                    event = pending;
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }
}

pub(super) fn descriptor(
    id: &str,
    version: String,
    voices: &[(&str, &str, Option<VoiceGender>)],
) -> EngineDescriptor {
    EngineDescriptor {
        id: id.to_owned(),
        display_name: format!("Linux {id}"),
        version: Some(version),
        availability: Availability::Available,
        health: EngineHealth::Healthy,
        capabilities: EngineCapabilities {
            acss: AcssCapabilities {
                rate: true,
                average_pitch: true,
                pitch_range: true,
                stress: true,
                richness: true,
                volume: true,
            },
            audio_output: AudioOutputMode::StreamingPcm,
            cancellation: CancellationSupport::SynthesisAndPlayback,
            concurrency: ConcurrencyModel::Serialized,
            markers: MarkerCapabilities::default(),
            language_switching: false,
            text_repertoire: TextRepertoire::Iso8859_1,
            post_synthesis_dimensions: buffered_post_synthesis_dimensions(),
            native_extensions: Vec::new(),
        },
        default_voice_id: Some(voices[0].0.to_owned()),
        voices: voices
            .iter()
            .map(|(voice, name, gender)| VoiceDescriptor {
                id: PhysicalVoiceId::new(id, *voice),
                display_name: (*name).to_owned(),
                language: Some("en-US".to_owned()),
                gender: *gender,
                quality: VoiceQuality::Compact,
                availability: Availability::Available,
            })
            .collect(),
    }
}

pub(super) fn absolute_file(
    name: &str,
    candidates: &[impl AsRef<Path>],
) -> Result<PathBuf, String> {
    let path = if let Some(value) = std::env::var_os(name).filter(|v| !v.is_empty()) {
        PathBuf::from(value)
    } else {
        candidates
            .iter()
            .map(|path| path.as_ref().to_path_buf())
            .find(|p| p.is_file())
            .ok_or_else(|| format!("runtime file not found; set {name} to its absolute path"))?
    };
    if !path.is_absolute() || !path.is_file() {
        return Err(format!(
            "{name} must name an existing absolute file: {}",
            path.display()
        ));
    }
    path.canonicalize().map_err(|e| format!("{name}: {e}"))
}

pub(super) fn open_library(path: &Path) -> Result<Library, String> {
    let mut header = [0_u8; 20];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut header))
        .map_err(|e| e.to_string())?;
    validate_elf(&header)?;
    // Explicit absolute path, RTLD_LOCAL and eager symbol resolution. The
    // dynamic linker remains responsible for the installed runtime's dependencies.
    unsafe {
        libloading::os::unix::Library::open(
            Some(path),
            libloading::os::unix::RTLD_NOW | libloading::os::unix::RTLD_LOCAL,
        )
    }
    .map(Into::into)
    .map_err(|e| format!("could not load {}: {e}", path.display()))
}

fn validate_elf(header: &[u8; 20]) -> Result<(), String> {
    let machine = match std::env::consts::ARCH {
        "x86_64" => 62,
        "x86" => 3,
        "aarch64" => 183,
        "arm" => 40,
        _ => return Err("unsupported helper architecture".to_owned()),
    };
    if &header[..4] != b"\x7fELF"
        || header[4] != if usize::BITS == 64 { 2 } else { 1 }
        || header[5] != 1
        || u16::from_le_bytes([header[18], header[19]]) != machine
        || u16::from_le_bytes([header[16], header[17]]) != 3
    {
        return Err(format!("runtime must be a little-endian ELF shared library matching this {} helper; a 32-bit library needs a 32-bit helper", std::env::consts::ARCH));
    }
    Ok(())
}

pub(super) unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, String> {
    library.get::<T>(name).map(|s| *s).map_err(|e| {
        format!(
            "required native symbol {}: {e}",
            String::from_utf8_lossy(name)
        )
    })
}

pub(super) fn encode_text(text: &str) -> Result<CString, String> {
    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\0' {
            return Err("text contains a null byte".to_owned());
        }
        if u32::from(ch) <= 255 {
            bytes.push(ch as u8);
        } else {
            return Err(format!(
                "character {ch:?} is outside this runtime's ISO-8859-1 repertoire"
            ));
        }
    }
    CString::new(bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_preserves_latin1_and_rejects_unrepresentable_input() {
        assert_eq!(
            encode_text("café naïve").unwrap().as_bytes(),
            b"caf\xe9 na\xefve"
        );
        assert!(encode_text("日本").is_err());
        assert!(encode_text("curly ’ quote").is_err());
        assert!(encode_text("a\0b").is_err());
    }

    #[test]
    fn runtime_loader_rejects_windows_and_mismatched_elf_abis() {
        let mut header = [0_u8; 20];
        header[..4].copy_from_slice(b"\x7fELF");
        header[4] = if usize::BITS == 64 { 2 } else { 1 };
        header[5] = 1;
        header[16] = 3;
        header[18] = match std::env::consts::ARCH {
            "x86_64" => 62,
            "x86" => 3,
            "aarch64" => 183,
            "arm" => 40,
            _ => return,
        };
        assert!(validate_elf(&header).is_ok());
        let mut wrong = header;
        wrong[4] = if header[4] == 2 { 1 } else { 2 };
        assert!(validate_elf(&wrong).is_err());
        wrong = header;
        wrong[18] = 0;
        assert!(validate_elf(&wrong).is_err());
        wrong = header;
        wrong[5] = 2;
        assert!(validate_elf(&wrong).is_err());
        wrong = header;
        wrong[..2].copy_from_slice(b"MZ");
        assert!(validate_elf(&wrong).is_err());
    }

    #[test]
    fn stop_unblocks_full_native_pcm_queue_without_a_consumer() {
        let (sender, receiver) = sync_channel(1);
        let epoch = Arc::new(AtomicU64::new(0));
        let mut capture = Capture {
            sender,
            frames: 0,
            marker_count: 0,
            cancellation: Cancellation {
                epoch: epoch.clone(),
                expected: 0,
                request: None,
                aborted: Arc::new(AtomicBool::new(false)),
            },
        };
        capture.audio(&[100; 512]).unwrap();
        let (done, finished) = sync_channel(1);
        let producer = std::thread::spawn(move || {
            done.send(capture.audio(&[200; 512])).unwrap();
        });
        epoch.fetch_add(1, Ordering::AcqRel);
        let result = finished.recv_timeout(Duration::from_secs(1));
        drop(receiver); // Also retires the test worker if cancellation regresses.
        producer.join().unwrap();
        assert!(result
            .expect("cancelled callback must release backpressure")
            .is_err());
    }
}
