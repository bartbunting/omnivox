// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
#![cfg(target_os = "linux")]

use omnivox_tts::helper_protocol::*;
use omnivox_tts::{AnchorAffinity, RequestedAnchor};
use std::io::{BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omnivox-linux-abi-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let status = Command::new("cc")
            .args(["-std=c11", "-shared", "-fPIC", "-pthread"])
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/runtime_stub.c"))
            .arg("-o")
            .arg(path.join("runtime.so"))
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::write(path.join("dictionary.dic"), b"test dictionary").unwrap();
        Self(path)
    }
    fn session(&self, id: &str, bad: bool, missing: bool) -> Session {
        self.session_with_voxin(id, bad, missing, None)
    }
    fn session_with_voxin(
        &self,
        id: &str,
        bad: bool,
        missing: bool,
        voxin_patch: Option<&str>,
    ) -> Session {
        self.session_options(id, bad, missing, voxin_patch, None)
    }
    fn session_options(
        &self,
        id: &str,
        bad: bool,
        missing: bool,
        voxin_patch: Option<&str>,
        marker_fault: Option<&str>,
    ) -> Session {
        let executable = if id == "eloquence" {
            env!("CARGO_BIN_EXE_omnivox-eloquence-helper")
        } else {
            env!("CARGO_BIN_EXE_omnivox-dectalk-helper")
        };
        let mut command = Command::new(executable);
        let variable = if id == "eloquence" {
            "OMNIVOX_ECI_LIBRARY"
        } else {
            "OMNIVOX_DECTALK_LIBRARY"
        };
        command
            .env(
                variable,
                self.0
                    .join(if missing { "missing.so" } else { "runtime.so" }),
            )
            .env("OMNIVOX_DECTALK_DICTIONARY", self.0.join("dictionary.dic"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        command.env_remove("OMNIVOX_STUB_BAD_PCM");
        command.env_remove("OMNIVOX_STUB_VOXIN_PATCH");
        for variable in ["OMNIVOX_STUB_OMIT_INDEX", "OMNIVOX_STUB_DUPLICATE_INDEX"] {
            command.env_remove(variable);
        }
        if let Some(variable) = marker_fault {
            command.env(variable, "1");
        }
        if let Some(patch) = voxin_patch {
            command.env("OMNIVOX_STUB_VOXIN_PATCH", patch);
        }
        if bad {
            command.env("OMNIVOX_STUB_BAD_PCM", "1");
        }
        let mut child = command.spawn().unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (send, receive) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            while let Ok(Some(frame)) = read_frame::<_, HelperResponse>(&mut output) {
                if send.send(frame).is_err() {
                    break;
                }
            }
        });
        let mut session = Session {
            child,
            input,
            receive,
            reader: Some(reader),
        };
        session.send(
            1,
            HelperRequestBody::Hello {
                supported_protocol_versions: vec![5],
            },
        );
        assert!(matches!(
            session.next().body,
            HelperResponseBody::Hello { .. }
        ));
        session
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Session {
    child: Child,
    input: ChildStdin,
    receive: Receiver<HelperResponse>,
    reader: Option<std::thread::JoinHandle<()>>,
}
impl Session {
    fn send(&mut self, id: u64, body: HelperRequestBody) {
        write_frame(&mut self.input, &HelperRequest::with_version(5, id, body)).unwrap();
        self.input.flush().unwrap();
    }
    #[track_caller]
    fn next(&self) -> HelperResponse {
        let response = self
            .receive
            .recv_timeout(Duration::from_secs(5))
            .expect("helper must remain responsive");
        response.validate().unwrap();
        response
    }
    fn synthesize(&mut self, id: u64, voice: &str) {
        self.send(
            id,
            HelperRequestBody::Synthesize {
                text: "First café sentence. Another sentence follows.".to_owned(),
                settings: HelperSynthesisSettings {
                    voice_id: Some(voice.to_owned()),
                    rate: 0.5,
                    pitch: 1.0,
                    volume: 0.8,
                    pitch_range: None,
                    stress: None,
                    richness: None,
                },
                anchors: Some(Vec::new()),
            },
        );
    }
    fn completed(&self, id: u64) {
        let mut samples = 0;
        let mut sequence = 0;
        loop {
            let frame = self.next();
            assert_eq!(frame.request_id, Some(id));
            match frame.body {
                HelperResponseBody::SynthesisStarted { format, .. } => {
                    assert_eq!(format.sample_rate, 44100);
                    assert_eq!(format.channels, 2);
                }
                HelperResponseBody::AudioChunk { chunk } => {
                    assert_eq!(chunk.sequence, sequence);
                    sequence += 1;
                    let pcm = chunk.decode_samples().unwrap();
                    assert!(pcm.iter().any(|v| *v != 0));
                    samples += pcm.len();
                }
                HelperResponseBody::Markers { markers } => {
                    assert!(markers
                        .iter()
                        .all(|mark| mark.frame_offset >= samples as u64 / 2));
                }
                HelperResponseBody::SynthesisCompleted { frame_count } => {
                    assert_eq!(frame_count, samples as u64 / 2);
                    assert!(samples > 0);
                    break;
                }
                other => panic!("unexpected synthesis response: {other:?}"),
            }
        }
    }
    fn shutdown(&mut self) {
        self.send(1000, HelperRequestBody::Shutdown);
        assert!(matches!(self.next().body, HelperResponseBody::ShuttingDown));
        for _ in 0..100 {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("helper did not exit after shutdown");
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
fn voxin_clear_input_workaround_is_limited_to_the_verified_version() {
    let fixture = Fixture::new();
    let mut session = fixture.session_with_voxin("eloquence", false, false, Some("3"));
    session.synthesize(2, "v1");
    session.completed(2);
    session.synthesize(3, "v2");
    session.completed(3);
    session.shutdown();

    let mut session = fixture.session_with_voxin("eloquence", false, false, Some("4"));
    session.synthesize(2, "v1");
    loop {
        match session.next().body {
            HelperResponseBody::SynthesisStarted { .. } => {}
            HelperResponseBody::Error { message, .. } => {
                assert!(message.contains("clear input failed"));
                break;
            }
            other => panic!("unverified runtime failure was suppressed: {other:?}"),
        }
    }
    session.shutdown();
}

#[test]
fn both_native_abis_stream_cancel_and_resume() {
    let fixture = Fixture::new();
    for id in ["eloquence", "dectalk"] {
        eprintln!("Testing {id} streaming and cancellation");
        let mut session = fixture.session(id, false, false);
        session.send(2, HelperRequestBody::Describe);
        let HelperResponseBody::Descriptor { descriptor } = session.next().body else {
            panic!("missing descriptor")
        };
        assert!(descriptor.can_synthesize());
        assert_eq!(descriptor.id, id);
        let voice = descriptor.default_voice_id.unwrap();
        session.synthesize(3, &voice);
        loop {
            match session.next().body {
                HelperResponseBody::AudioChunk { .. } => break,
                HelperResponseBody::SynthesisStarted { .. }
                | HelperResponseBody::Markers { .. } => {}
                other => panic!("expected first PCM: {other:?}"),
            }
        }
        session.send(
            4,
            HelperRequestBody::Cancel {
                target_request_id: 3,
            },
        );
        let mut accepted = false;
        let mut cancelled = false;
        while !accepted || !cancelled {
            match session.next().body {
                HelperResponseBody::CancelAccepted { .. } => accepted = true,
                HelperResponseBody::SynthesisCancelled => cancelled = true,
                HelperResponseBody::AudioChunk { .. } | HelperResponseBody::Markers { .. } => {}
                other => panic!("unexpected cancellation: {other:?}"),
            }
        }
        for (index, voice) in descriptor.voices.iter().enumerate() {
            eprintln!("Testing {id}/{} after cancellation", voice.id.voice_id);
            let request_id = 10 + index as u64;
            session.synthesize(request_id, &voice.id.voice_id);
            session.completed(request_id);
        }
        session.shutdown();
    }
}

#[test]
fn missing_libraries_remain_protocol_unavailability() {
    let fixture = Fixture::new();
    for id in ["eloquence", "dectalk"] {
        let mut session = fixture.session(id, false, true);
        session.send(2, HelperRequestBody::Describe);
        assert!(matches!(
            session.next().body,
            HelperResponseBody::Error {
                code: HelperErrorCode::NotAvailable,
                ..
            }
        ));
        session.synthesize(3, if id == "eloquence" { "v1" } else { "paul" });
        assert!(matches!(
            session.next().body,
            HelperResponseBody::Error { .. }
        ));
        session.shutdown();
    }
}

#[test]
fn invalid_callback_lengths_fail_without_crashing_the_helper() {
    let fixture = Fixture::new();
    for id in ["eloquence", "dectalk"] {
        let mut session = fixture.session(id, true, false);
        session.synthesize(3, if id == "eloquence" { "v1" } else { "paul" });
        loop {
            match session.next().body {
                HelperResponseBody::SynthesisStarted { .. }
                | HelperResponseBody::Markers { .. } => {}
                HelperResponseBody::Error { .. } => break,
                other => panic!("invalid native PCM accepted: {other:?}"),
            }
        }
        session.send(4, HelperRequestBody::Ping);
        assert!(matches!(session.next().body, HelperResponseBody::Pong));
        session.shutdown();
    }
}

#[test]
fn native_markers_keep_utf8_positions_and_precede_audio_even_with_late_dectalk_callbacks() {
    let fixture = Fixture::new();
    for engine in ["eloquence", "dectalk"] {
        let mut session = fixture.session(engine, false, false);
        for id in [2, 3] {
            session.send(
                id,
                HelperRequestBody::Synthesize {
                    text: "Café next.".to_owned(),
                    settings: HelperSynthesisSettings {
                        voice_id: Some(
                            if engine == "eloquence" { "v1" } else { "paul" }.to_owned(),
                        ),
                        rate: 0.5,
                        pitch: 1.0,
                        volume: 0.8,
                        pitch_range: Some(0.8),
                        stress: Some(0.2),
                        richness: Some(0.7),
                    },
                    anchors: Some(vec![
                        RequestedAnchor::new("begin", 0, AnchorAffinity::Before),
                        RequestedAnchor::new("next", 6, AnchorAffinity::Before),
                        RequestedAnchor::new("same", 6, AnchorAffinity::After),
                    ]),
                },
            );
            let mut published = 0;
            let mut marks = Vec::new();
            let mut last_marker = 0;
            loop {
                match session.next().body {
                    HelperResponseBody::SynthesisStarted { .. } => {}
                    HelperResponseBody::AudioChunk { chunk } => {
                        published += chunk.decode_samples().unwrap().len() as u64 / 2;
                    }
                    HelperResponseBody::Markers { markers } => {
                        for marker in &markers {
                            assert!(
                                marker.frame_offset >= published,
                                "{engine}: marker behind audio"
                            );
                            assert!(
                                marker.frame_offset >= last_marker,
                                "{engine}: reordered markers"
                            );
                            last_marker = marker.frame_offset;
                        }
                        marks.extend(markers);
                    }
                    HelperResponseBody::SynthesisCompleted { frame_count } => {
                        assert_eq!(frame_count, published);
                        assert!(marks.iter().all(|m| m.frame_offset <= frame_count));
                        break;
                    }
                    other => panic!("{engine}: unexpected anchored response {other:?}"),
                }
            }
            let anchors: Vec<_> = marks
                .iter()
                .filter(|m| m.kind == HelperMarkerKind::RequestedAnchor)
                .collect();
            assert_eq!(anchors.len(), 3);
            for anchor in anchors {
                assert_eq!(
                    anchor.frame_offset,
                    if anchor.value.as_deref() == Some("begin") {
                        0
                    } else {
                        640
                    }
                );
            }
            assert!(marks.iter().any(|m| m.kind == HelperMarkerKind::Word
                && m.text_start == Some(6)
                && m.text_length == Some(4)));
            assert!(marks.iter().any(|m| m.kind == HelperMarkerKind::Sentence));
            if engine == "dectalk" {
                assert!(marks.iter().any(|m| m.kind == HelperMarkerKind::Phoneme));
            }
        }
        session.shutdown();
    }
}

#[test]
fn missing_or_duplicate_native_anchors_fail_without_retiring_the_protocol() {
    let fixture = Fixture::new();
    for fault in ["OMNIVOX_STUB_OMIT_INDEX", "OMNIVOX_STUB_DUPLICATE_INDEX"] {
        let mut session = fixture.session_options("eloquence", false, false, None, Some(fault));
        session.send(
            2,
            HelperRequestBody::Synthesize {
                text: "Café next.".to_owned(),
                settings: HelperSynthesisSettings {
                    voice_id: Some("v1".to_owned()),
                    rate: 0.5,
                    pitch: 1.0,
                    volume: 1.0,
                    pitch_range: None,
                    stress: None,
                    richness: None,
                },
                anchors: Some(vec![RequestedAnchor::new(
                    "test",
                    6,
                    AnchorAffinity::Before,
                )]),
            },
        );
        loop {
            match session.next().body {
                HelperResponseBody::SynthesisStarted { .. }
                | HelperResponseBody::AudioChunk { .. }
                | HelperResponseBody::Markers { .. } => {}
                HelperResponseBody::Error { message, .. } => {
                    assert!(message.contains(if fault == "OMNIVOX_STUB_OMIT_INDEX" {
                        "omitted"
                    } else {
                        "duplicate"
                    }));
                    break;
                }
                other => panic!("invalid native marker was accepted: {other:?}"),
            }
        }
        session.send(3, HelperRequestBody::Ping);
        assert!(matches!(session.next().body, HelperResponseBody::Pong));
        session.shutdown();
    }
}
