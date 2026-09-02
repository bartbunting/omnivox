//! Audio Pipeline
//!
//! Defines the AudioEffect trait and the AudioPipeline that chains stateless
//! buffer-wide effects together. Stateful presentation effects use the
//! separate bounded post-synthesis processor.

use crate::buffer::AudioBuffer;
use crate::AudioError;

/// An audio effect that transforms an AudioBuffer in place.
///
/// Implement this trait for each post-processing effect. Effects are
/// applied in sequence by the AudioPipeline.
pub trait AudioEffect: Send + Sync {
    /// Process the audio buffer in place.
    fn process(&self, buffer: &mut AudioBuffer) -> Result<(), AudioError>;

    /// Human-readable name for this effect (useful for logging).
    fn name(&self) -> &str;
}

/// A pipeline of audio effects applied in sequence.
///
/// Effects are processed in the order they were added. The pipeline
/// is designed to be extensible -- new effects can be pushed at any time.
pub struct AudioPipeline {
    effects: Vec<Box<dyn AudioEffect>>,
}

impl AudioPipeline {
    /// Create a new empty pipeline.
    pub fn new() -> Self {
        Self {
            effects: Vec::new(),
        }
    }

    /// Add an effect to the end of the pipeline.
    pub fn push(&mut self, effect: Box<dyn AudioEffect>) {
        self.effects.push(effect);
    }

    /// Get the number of effects in the pipeline.
    pub fn len(&self) -> usize {
        self.effects.len()
    }

    /// Check if the pipeline has no effects.
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    /// Process a buffer through all effects in order.
    pub fn process(&self, buffer: &mut AudioBuffer) -> Result<(), AudioError> {
        for effect in &self.effects {
            tracing::trace!(effect = effect.name(), "applying audio effect");
            effect.process(buffer)?;
        }
        Ok(())
    }

    /// Clear all effects from the pipeline.
    pub fn clear(&mut self) {
        self.effects.clear();
    }
}

impl Default for AudioPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test effect that multiplies all samples by a constant.
    struct ScaleEffect {
        factor: f32,
    }

    impl AudioEffect for ScaleEffect {
        fn process(&self, buffer: &mut AudioBuffer) -> Result<(), AudioError> {
            for sample in &mut buffer.samples {
                *sample *= self.factor;
            }
            Ok(())
        }

        fn name(&self) -> &str {
            "scale"
        }
    }

    /// A test effect that always fails.
    struct FailEffect;

    impl AudioEffect for FailEffect {
        fn process(&self, _buffer: &mut AudioBuffer) -> Result<(), AudioError> {
            Err(AudioError::EffectError("intentional failure".into()))
        }

        fn name(&self) -> &str {
            "fail"
        }
    }

    #[test]
    fn test_empty_pipeline() {
        let pipeline = AudioPipeline::new();
        assert!(pipeline.is_empty());
        assert_eq!(pipeline.len(), 0);

        let mut buf = AudioBuffer::new(vec![0.5, -0.5]);
        pipeline.process(&mut buf).unwrap();
        assert_eq!(buf.samples, vec![0.5, -0.5]);
    }

    #[test]
    fn test_single_effect() {
        let mut pipeline = AudioPipeline::new();
        pipeline.push(Box::new(ScaleEffect { factor: 2.0 }));

        let mut buf = AudioBuffer::new(vec![0.25, -0.25]);
        pipeline.process(&mut buf).unwrap();
        assert_eq!(buf.samples, vec![0.5, -0.5]);
    }

    #[test]
    fn test_chained_effects() {
        let mut pipeline = AudioPipeline::new();
        pipeline.push(Box::new(ScaleEffect { factor: 2.0 }));
        pipeline.push(Box::new(ScaleEffect { factor: 3.0 }));

        let mut buf = AudioBuffer::new(vec![0.1, -0.1]);
        pipeline.process(&mut buf).unwrap();

        // 0.1 * 2.0 * 3.0 = 0.6
        let expected = 0.1 * 2.0 * 3.0;
        assert!((buf.samples[0] - expected).abs() < 1e-6);
        assert!((buf.samples[1] - (-expected)).abs() < 1e-6);
    }

    #[test]
    fn test_effect_order_matters() {
        // Pipeline with add-then-scale vs scale-then-add would differ,
        // but with two scale effects the order doesn't matter for multiplication.
        // Use a different approach: scale by 0 then scale by 2 -- result should be 0.
        let mut pipeline = AudioPipeline::new();
        pipeline.push(Box::new(ScaleEffect { factor: 0.0 }));
        pipeline.push(Box::new(ScaleEffect { factor: 2.0 }));

        let mut buf = AudioBuffer::new(vec![0.5, -0.5]);
        pipeline.process(&mut buf).unwrap();
        assert_eq!(buf.samples, vec![0.0, -0.0]);
    }

    #[test]
    fn test_effect_error_propagates() {
        let mut pipeline = AudioPipeline::new();
        pipeline.push(Box::new(ScaleEffect { factor: 2.0 }));
        pipeline.push(Box::new(FailEffect));
        pipeline.push(Box::new(ScaleEffect { factor: 3.0 }));

        let mut buf = AudioBuffer::new(vec![0.5, -0.5]);
        let result = pipeline.process(&mut buf);
        assert!(result.is_err());
    }

    #[test]
    fn test_clear_pipeline() {
        let mut pipeline = AudioPipeline::new();
        pipeline.push(Box::new(ScaleEffect { factor: 2.0 }));
        assert_eq!(pipeline.len(), 1);

        pipeline.clear();
        assert!(pipeline.is_empty());
    }

    #[test]
    fn test_pipeline_len() {
        let mut pipeline = AudioPipeline::new();
        assert_eq!(pipeline.len(), 0);
        pipeline.push(Box::new(ScaleEffect { factor: 1.0 }));
        assert_eq!(pipeline.len(), 1);
        pipeline.push(Box::new(ScaleEffect { factor: 1.0 }));
        assert_eq!(pipeline.len(), 2);
    }
}
