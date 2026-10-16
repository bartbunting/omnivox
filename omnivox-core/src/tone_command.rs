//! Versioned standalone presentation tones.

use crate::queue::TonePlacement;
use crate::tone::{AdsrEnvelope, EnvelopeCurve, ToneParameters, ToneShape, Waveform};

pub const RICH_PRESENTATION_TONE_PROTOCOL_VERSION: u8 = 2;

/// Decode without changing the legacy version-one parser or its bounds.
pub fn parse_presentation_tone_item(arguments: &str) -> Result<crate::queue::QueueItem, String> {
    if matches!(arguments.split_whitespace().next(), Some("2" | "3")) {
        let command = parse_rich_presentation_tone_arguments(arguments)?;
        Ok(crate::queue::QueueItem::ShapedTone {
            tone: command.tone,
            placement: command.placement,
        })
    } else {
        let command = crate::command::parse_presentation_tone_arguments(arguments)?;
        Ok(crate::queue::QueueItem::Tone {
            frequency: command.frequency_hz,
            duration: command.duration_ms,
            placement: command.placement,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RichPresentationToneCommand {
    pub placement: TonePlacement,
    pub tone: ToneParameters,
}

/// Decode `2 MODE FREQUENCY DURATION GAIN WAVEFORM classic`, or the ADSR
/// form ending in `adsr ATTACK DECAY SUSTAIN RELEASE`. Exact arity is required.
pub fn parse_rich_presentation_tone_arguments(
    arguments: &str,
) -> Result<RichPresentationToneCommand, String> {
    // Bound allocation even when called outside the bounded protocol reader.
    let all_fields = arguments.split_whitespace().take(15).collect::<Vec<_>>();
    let extended = all_fields.first() == Some(&"3");
    let fields = if extended && matches!(all_fields.len(), 10 | 14) {
        &all_fields[..all_fields.len() - 3]
    } else {
        &all_fields[..]
    };
    if fields.len() != 7 && fields.len() != 11 {
        return Err("rich tone requires 7 Classic or 11 ADSR fields".into());
    }
    if fields[0] != "2" && !extended {
        return Err("rich tone requires version 2".into());
    }
    let placement = match fields[1] {
        "insert" => TonePlacement::Insert,
        "overlay" => TonePlacement::Overlay,
        _ => return Err("rich tone mode must be insert or overlay".into()),
    };
    let waveform = match fields[5] {
        "sine" => Waveform::Sine,
        "triangle" => Waveform::Triangle,
        "square" => Waveform::Square,
        "pulse" if extended => Waveform::Pulse,
        "sawtooth" if extended => Waveform::Sawtooth,
        _ => return Err("unknown tone waveform".into()),
    };
    let envelope = match fields[6] {
        "classic" if fields.len() == 7 => None,
        "adsr" if fields.len() == 11 => Some(AdsrEnvelope {
            attack_ms: fields[7].parse().map_err(|_| "invalid attack time")?,
            decay_ms: fields[8].parse().map_err(|_| "invalid decay time")?,
            sustain_level: fields[9].parse().map_err(|_| "invalid sustain level")?,
            release_ms: fields[10].parse().map_err(|_| "invalid release time")?,
        }),
        _ => return Err("invalid tone envelope or field count".into()),
    };
    let mut shape = ToneShape {
        waveform,
        envelope,
        ..ToneShape::default()
    };
    if extended {
        let tail = &all_fields[fields.len()..];
        if tail.len() != 3 {
            return Err("extended tone requires width, sweep end and curve fields".into());
        }
        let number = |value: &str| -> Result<Option<f32>, String> {
            if value == "-" {
                Ok(None)
            } else {
                value
                    .parse()
                    .map(Some)
                    .map_err(|_| "invalid tone extension number".into())
            }
        };
        shape.pulse_width = number(tail[0])?;
        shape.end_frequency_hz = number(tail[1])?;
        shape.envelope_curve = match tail[2] {
            "-" => None,
            "linear" => Some(EnvelopeCurve::Linear),
            "exponential" => Some(EnvelopeCurve::Exponential),
            _ => return Err("unknown envelope curve".into()),
        };
    }
    let tone = ToneParameters {
        frequency_hz: fields[2].parse().map_err(|_| "invalid tone frequency")?,
        duration_ms: fields[3].parse().map_err(|_| "invalid tone duration")?,
        volume: fields[4].parse().map_err(|_| "invalid tone volume")?,
        shape,
    };
    tone.validate()?;
    Ok(RichPresentationToneCommand { placement, tone })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::parse_presentation_tone_arguments;

    #[test]
    fn extended_tones_validate_options_without_widening_old_versions() {
        for (wave, width) in [("pulse", "0.25"), ("sawtooth", "-")] {
            let request =
                format!("3 overlay 440 100 0.35 {wave} adsr 2 30 0.4 10 {width} 880 exponential");
            let parsed = parse_rich_presentation_tone_arguments(&request).unwrap();
            assert_eq!(parsed.tone.shape.end_frequency_hz, Some(880.0));
            assert!(parse_presentation_tone_item(&request).is_ok());
            assert!(
                parse_rich_presentation_tone_arguments(&request.replacen("3 ", "2 ", 1)).is_err()
            );
        }
        for request in [
            "3 insert 440 50 1 pulse classic - - -",
            "3 insert 440 50 1 pulse classic 0.01 - -",
            "3 insert 440 50 1 pulse classic NaN - -",
            "3 insert 440 50 1 sine classic 0.5 - -",
            "3 insert 440 50 1 sine classic - NaN -",
            "3 insert 440 50 1 sine classic - 20001 -",
            "3 insert 440 50 1 sine classic - - exponential",
            "3 insert 440 50 1 sine adsr 2 5 0.5 5 - - unknown",
            "3 insert 440 50 1 sawtooth classic - - - extra",
        ] {
            assert!(parse_presentation_tone_item(request).is_err(), "{request}");
        }
    }

    #[test]
    fn queue_decoder_preserves_old_items_and_freezes_rich_parameters() {
        let old = parse_presentation_tone_item("1 insert 440 50").unwrap();
        assert_eq!(
            old,
            crate::queue::QueueItem::Tone {
                frequency: 440.0,
                duration: 50,
                placement: TonePlacement::Insert
            }
        );
        let request = "2 overlay 880 40 0.35 triangle adsr 1 25 0.25 10";
        let expected = parse_rich_presentation_tone_arguments(request).unwrap();
        assert_eq!(
            parse_presentation_tone_item(request).unwrap(),
            crate::queue::QueueItem::ShapedTone {
                tone: expected.tone,
                placement: expected.placement
            }
        );
    }

    #[test]
    fn preserves_modes_gain_and_short_envelopes_without_normalizing() {
        let tone = parse_rich_presentation_tone_arguments(
            "2 insert 725.5 1 0.25 triangle adsr 60000 60000 0.4 60000",
        )
        .unwrap();
        assert_eq!(tone.placement, TonePlacement::Insert);
        assert_eq!(tone.tone.frequency_hz, 725.5);
        assert_eq!(tone.tone.duration_ms, 1);
        assert_eq!(tone.tone.volume, 0.25);
        assert_eq!(tone.tone.shape.waveform, Waveform::Triangle);
        assert_eq!(tone.tone.shape.envelope.unwrap().release_ms, 60000);
        assert_eq!(
            parse_rich_presentation_tone_arguments("2 overlay 24000 60000 0 sine classic")
                .unwrap()
                .tone
                .shape,
            ToneShape::default()
        );
        assert!(parse_presentation_tone_arguments("1 overlay 24000 60000").is_ok());
        assert!(parse_presentation_tone_arguments("2 overlay 440 50 1 sine classic").is_err());
    }

    #[test]
    fn rejects_malformed_and_out_of_bounds_parameters() {
        for request in [
            "1 insert 440 50 1 sine classic",
            "3 insert 440 50 1 sine classic",
            "2 independent 440 50 1 sine classic",
            "2 insert NaN 50 1 sine classic",
            "2 insert inf 50 1 sine classic",
            "2 insert 0 50 1 sine classic",
            "2 insert 24001 50 1 sine classic",
            "2 insert 20001 50 1 triangle classic",
            "2 insert 24000 50 1 sine adsr 0 0 1 0",
            "2 insert 440 0 1 sine classic",
            "2 insert 440 60001 1 sine classic",
            "2 insert 440 1.5 1 sine classic",
            "2 insert 440 50 NaN sine classic",
            "2 insert 440 50 -0.1 sine classic",
            "2 insert 440 50 1.1 sine classic",
            "2 insert 440 50 1 saw classic",
            "2 insert 440 50 1 sine unknown",
            "2 insert 440 50 1 sine classic extra",
            "2 insert 440 50 1 sine adsr",
            "2 insert 440 50 1 sine adsr -1 0 1 0",
            "2 insert 440 50 1 sine adsr 60001 0 1 0",
            "2 insert 440 50 1 sine adsr 0 0 NaN 0",
            "2 insert 440 50 1 sine adsr 0 0 1.1 0",
            "2 insert 440 50 1 sine adsr 0 0 1 0 trailing",
            "2 insert 440 50 1 sine classic 0 0 1 0",
        ] {
            assert!(
                parse_rich_presentation_tone_arguments(request).is_err(),
                "{request}"
            );
        }
    }
}
