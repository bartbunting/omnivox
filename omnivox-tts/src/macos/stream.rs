//! Request-local PCM conversion, shared by native streaming and collection.

use crate::contracts::{AnchorSupport, PhysicalVoiceId};
use crate::{
    AudioBuffer, SynthesisRequest, SynthesisResult, SynthesisStreamCompletion, SynthesisStreamSink,
    SynthesisStreamStart, TtsError,
};
use omnivox_audio::ProgressivePcmCanonicalizer;

// Bound the full-result compatibility collector as well as cumulative native
// output. Reuse the helper synthesis budget without changing its wire contract.
const MAX_SAMPLES: usize =
    crate::helper_protocol::MAX_HELPER_SYNTHESIS_BYTES / std::mem::size_of::<f32>();
pub(super) const WINDOW_SAMPLES: usize = 1024;

pub(super) fn failed(message: impl std::fmt::Display) -> TtsError {
    TtsError::SynthesisFailed(format!("macOS synthesis: {message}"))
}

pub(super) struct Stream<'a> {
    request: &'a SynthesisRequest,
    sink: &'a mut dyn SynthesisStreamSink,
    actual_voice: Option<PhysicalVoiceId>,
    format: Option<(u32, u16)>,
    converter: Option<ProgressivePcmCanonicalizer>,
    samples: usize,
    stop_check: Option<&'a dyn Fn() -> bool>,
}

impl<'a> Stream<'a> {
    pub(super) fn new(
        request: &'a SynthesisRequest,
        actual_voice: Option<PhysicalVoiceId>,
        sink: &'a mut dyn SynthesisStreamSink,
    ) -> Self {
        Self {
            request,
            sink,
            actual_voice,
            format: None,
            converter: None,
            samples: 0,
            stop_check: None,
        }
    }

    pub(super) fn with_stop_check(mut self, check: &'a dyn Fn() -> bool) -> Self {
        self.stop_check = Some(check);
        self
    }

    pub(super) fn push(
        &mut self,
        samples: &[f32],
        rate: u32,
        channels: u16,
    ) -> Result<(), TtsError> {
        self.check_cancelled()?;
        if samples.is_empty() || samples.len() > WINDOW_SAMPLES {
            return Err(failed("invalid native window size"));
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(failed("non-finite native PCM"));
        }
        if samples.len() > MAX_SAMPLES.saturating_sub(self.samples) {
            return Err(failed("native PCM exceeded the synthesis limit"));
        }
        self.samples += samples.len();
        if let Some(format) = self.format {
            if format != (rate, channels) {
                return Err(failed("native PCM format changed within one utterance"));
            }
        } else {
            self.converter =
                Some(ProgressivePcmCanonicalizer::new(rate, channels).map_err(failed)?);
            self.format = Some((rate, channels));
            self.sink.start(SynthesisStreamStart {
                engine_id: "macos".into(),
                actual_voice: self.actual_voice.clone(),
                degraded_acss: self
                    .request
                    .normalized_acss
                    .clone()
                    .degrade_for(&super::macos_capabilities().acss)
                    .omitted,
            })?;
        }
        let windows = self
            .converter
            .as_mut()
            .expect("initialized converter")
            .push_interleaved_f32(samples)
            .map_err(failed)?;
        self.emit(windows)
    }

    fn check_cancelled(&self) -> Result<(), TtsError> {
        if self.stop_check.is_some_and(|check| check())
            || self
                .request
                .cancellation
                .as_ref()
                .is_some_and(|token| token.is_cancelled())
        {
            Err(failed("cancelled"))
        } else {
            Ok(())
        }
    }

    fn emit(&mut self, windows: Vec<AudioBuffer>) -> Result<(), TtsError> {
        for window in windows {
            self.check_cancelled()?;
            if !window.is_empty() {
                self.sink.audio(window)?;
            }
        }
        self.check_cancelled()
    }

    pub(super) fn finish(mut self) -> Result<SynthesisStreamCompletion, TtsError> {
        self.check_cancelled()?;
        let converter = self
            .converter
            .as_mut()
            .ok_or_else(|| failed("no audio received"))?;
        let windows = converter.finish().map_err(failed)?;
        let frame_count = converter.output_frames();
        self.emit(windows)?;
        // No source-accurate marker support is claimed by this adapter.
        let mut metadata =
            SynthesisResult::audio("macos", self.actual_voice.clone(), AudioBuffer::empty());
        metadata.resolve_anchors(self.request, AnchorSupport::None);
        if !metadata.anchors.is_empty() {
            self.sink.markers(Vec::new(), metadata.anchors)?;
        }
        self.check_cancelled()?;
        Ok(SynthesisStreamCompletion { frame_count })
    }
}

#[derive(Default)]
pub(super) struct Collector {
    pub(super) result: Option<SynthesisResult>,
}

impl SynthesisStreamSink for Collector {
    fn start(&mut self, start: SynthesisStreamStart) -> Result<(), TtsError> {
        if self.result.is_some() {
            return Err(failed("duplicate stream start"));
        }
        let mut result =
            SynthesisResult::audio(start.engine_id, start.actual_voice, AudioBuffer::empty());
        result.degraded_acss = start.degraded_acss;
        self.result = Some(result);
        Ok(())
    }
    fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
        let result = self
            .result
            .as_mut()
            .ok_or_else(|| failed("audio before stream start"))?;
        if audio.samples.len() > MAX_SAMPLES.saturating_sub(result.audio.samples.len()) {
            return Err(failed("collected PCM exceeded the synthesis limit"));
        }
        result
            .audio
            .samples
            .try_reserve(audio.samples.len())
            .map_err(failed)?;
        result.audio.samples.extend(audio.samples);
        Ok(())
    }
    fn markers(
        &mut self,
        markers: Vec<crate::SynthesisMarker>,
        anchors: Vec<crate::ResolvedAnchor>,
    ) -> Result<(), TtsError> {
        let result = self
            .result
            .as_mut()
            .ok_or_else(|| failed("metadata before stream start"))?;
        result.markers.extend(markers);
        result.anchors.extend(anchors);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AnchorAffinity, AnchorResolution, RequestedAnchor, SynthesisCancellationToken, TtsSettings,
    };

    #[test]
    fn callback_boundaries_do_not_change_pcm_or_frame_count() {
        let request = SynthesisRequest::new("test", TtsSettings::default());
        let samples: Vec<f32> = (0..4097).map(|n| (n as f32 / 12.0).sin() * 0.2).collect();
        let collect = |size| {
            let mut collector = Collector::default();
            let mut stream = Stream::new(&request, None, &mut collector);
            for window in samples.chunks(size) {
                stream.push(window, 22050, 1).unwrap();
            }
            let completion = stream.finish().unwrap();
            let result = collector.result.unwrap();
            assert_eq!(completion.frame_count, 8194);
            assert_eq!(result.audio.frame_count(), 8194);
            result.audio.samples
        };
        assert_eq!(collect(97), collect(512));
    }

    #[test]
    fn audio_is_emitted_before_native_completion_and_anchors_remain_omitted() {
        let request = SynthesisRequest::new("test", TtsSettings::default())
            .with_anchors(vec![RequestedAnchor::new(
                "anchor",
                0,
                AnchorAffinity::Before,
            )])
            .unwrap();
        let mut collector = Collector::default();
        let mut stream = Stream::new(&request, None, &mut collector);
        stream.push(&[0.25; 512], 44100, 1).unwrap();
        stream.finish().unwrap();
        let result = collector.result.unwrap();
        assert_eq!(result.audio.frame_count(), 512);
        assert_eq!(result.anchors[0].resolution, AnchorResolution::Omitted);
        assert_eq!(result.anchors[0].frame_offset, None);
    }

    #[test]
    fn format_change_and_invalid_pcm_cannot_be_reported_as_success() {
        let request = SynthesisRequest::new("test", TtsSettings::default());
        let mut collector = Collector::default();
        let mut stream = Stream::new(&request, None, &mut collector);
        stream.push(&[0.25; 512], 22050, 1).unwrap();
        assert!(stream.push(&[0.25; 512], 44100, 1).is_err());
        assert!(stream.push(&[f32::NAN], 22050, 1).is_err());
        assert!(stream.push(&[0.0; WINDOW_SAMPLES + 1], 22050, 1).is_err());
    }

    #[test]
    fn cancelled_request_cannot_start_or_flush_a_tail() {
        let token = SynthesisCancellationToken::new();
        let request =
            SynthesisRequest::new("test", TtsSettings::default()).with_cancellation(token.clone());
        let mut collector = Collector::default();
        let mut stream = Stream::new(&request, None, &mut collector);
        stream.push(&[0.25; 97], 22050, 1).unwrap();
        token.cancel();
        assert!(stream.finish().is_err());
        assert!(collector.result.unwrap().audio.is_empty());
        let mut collector = Collector::default();
        assert!(Stream::new(&request, None, &mut collector)
            .push(&[0.25; 512], 44100, 1)
            .is_err());
        assert!(collector.result.is_none());
    }

    #[test]
    fn stop_inside_audio_sink_prevents_more_windows_and_success() {
        use std::cell::Cell;
        struct StopSink<'a>(&'a Cell<bool>, &'a Cell<usize>);
        impl SynthesisStreamSink for StopSink<'_> {
            fn start(&mut self, _: SynthesisStreamStart) -> Result<(), TtsError> {
                Ok(())
            }
            fn audio(&mut self, _: AudioBuffer) -> Result<(), TtsError> {
                self.1.set(self.1.get() + 1);
                self.0.set(true);
                Ok(())
            }
            fn markers(
                &mut self,
                _: Vec<crate::SynthesisMarker>,
                _: Vec<crate::ResolvedAnchor>,
            ) -> Result<(), TtsError> {
                panic!("metadata after stop")
            }
        }
        let stopped = Cell::new(false);
        let windows = Cell::new(0);
        let request = SynthesisRequest::new("test", TtsSettings::default());
        let mut sink = StopSink(&stopped, &windows);
        let check = || stopped.get();
        let mut stream = Stream::new(&request, None, &mut sink).with_stop_check(&check);
        assert!(stream.push(&[0.25; 1024], 8000, 1).is_err());
        assert_eq!(windows.get(), 1);
        assert!(stream.finish().is_err());
    }
}
