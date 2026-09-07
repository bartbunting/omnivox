// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later

//! Text bookmarks only acquire times from native ECI/DECtalk callbacks.
use omnivox_tts::*;
use std::collections::HashMap;

pub(super) const MAX_MARKERS: usize = 4096;

#[derive(Clone, Debug)]
pub(super) enum Mark {
    Text(SynthesisMarker),
    Anchor(ResolvedAnchor),
}
impl Mark {
    pub(super) fn frame(&self) -> u64 {
        match self {
            Self::Text(mark) => mark.frame_offset,
            Self::Anchor(mark) => mark.frame_offset.expect("native anchor has a frame"),
        }
    }
    pub(super) fn at(mut self, frame: u64) -> Self {
        match &mut self {
            Self::Text(mark) => mark.frame_offset = frame,
            Self::Anchor(mark) => mark.frame_offset = Some(frame),
        }
        self
    }
}

pub(super) struct Plan {
    pub(super) insertions: Vec<(usize, u32)>,
    pending: HashMap<u32, Vec<Mark>>,
    pub(super) leading: Vec<Mark>,
    pub(super) trailing: Vec<Mark>,
    pub(super) count: usize,
}

impl Plan {
    pub(super) fn new(request: &SynthesisRequest, exact: bool) -> Result<Self, String> {
        // Also validate callers using the engine directly, outside the wire host.
        request
            .clone()
            .with_anchors(request.anchors.clone())
            .map_err(|e| e.to_string())?;
        let mut entries: Vec<(usize, u8, Mark)> = Vec::new();
        let limit = MAX_MARKERS - request.anchors.len();
        for (kind, spans) in [
            (SynthesisMarkerKind::Sentence, sentences(&request.text)),
            (SynthesisMarkerKind::Word, words(&request.text)),
        ] {
            for (start, end) in spans {
                if entries.len() == limit {
                    break;
                }
                entries.push((
                    start,
                    if kind == SynthesisMarkerKind::Sentence {
                        1
                    } else {
                        2
                    },
                    Mark::Text(SynthesisMarker {
                        kind,
                        frame_offset: 0,
                        text_start: Some(start as u32),
                        text_length: Some((end - start) as u32),
                        value: (kind == SynthesisMarkerKind::Word && end - start <= 16 * 1024)
                            .then(|| request.text[start..end].to_owned()),
                    }),
                ));
            }
        }
        if exact {
            for anchor in &request.anchors {
                entries.push((
                    anchor.text_offset as usize,
                    if anchor.affinity == AnchorAffinity::Before {
                        0
                    } else {
                        3
                    },
                    anchor_mark(anchor, AnchorResolution::Exact),
                ));
            }
        }
        entries.sort_by_key(|(position, priority, _)| (*position, *priority));
        let mut plan = Self {
            insertions: Vec::new(),
            pending: HashMap::new(),
            leading: Vec::new(),
            trailing: Vec::new(),
            count: entries.len(),
        };
        let mut word_indexes = Vec::new();
        for (sequence, (position, _, mark)) in entries.into_iter().enumerate() {
            // DECtalk has a signed 16-bit index field. Reserve the same range
            // as Windows; plain user text cannot inject an index command.
            let index = 32767 - sequence as u32;
            if matches!(&mark, Mark::Text(m) if m.kind == SynthesisMarkerKind::Word) {
                word_indexes.push((position, index));
            }
            plan.insertions.push((position, index));
            plan.pending.insert(index, vec![mark]);
        }
        if !exact {
            for anchor in &request.anchors {
                let offset = anchor.text_offset as usize;
                let selected = if anchor.affinity == AnchorAffinity::Before {
                    word_indexes.iter().find(|(start, _)| *start >= offset)
                } else {
                    word_indexes
                        .iter()
                        .rev()
                        .find(|(start, _)| *start <= offset)
                };
                if let Some((_, index)) = selected {
                    plan.pending
                        .get_mut(index)
                        .unwrap()
                        .push(anchor_mark(anchor, AnchorResolution::WordBoundary));
                } else if anchor.affinity == AnchorAffinity::Before {
                    plan.leading
                        .push(anchor_mark(anchor, AnchorResolution::SpanBoundary));
                } else {
                    plan.trailing
                        .push(anchor_mark(anchor, AnchorResolution::SpanBoundary));
                }
                plan.count += 1;
            }
        }
        Ok(plan)
    }

    pub(super) fn reached(&mut self, index: u32, frame: u64) -> Result<Vec<Mark>, String> {
        self.pending
            .remove(&index)
            .ok_or_else(|| format!("native engine returned an unknown or duplicate index {index}"))
            .map(|marks| marks.into_iter().map(|mark| mark.at(frame)).collect())
    }

    pub(super) fn finish(&self) -> Result<(), String> {
        if self
            .pending
            .values()
            .flatten()
            .any(|mark| matches!(mark, Mark::Anchor(_)))
        {
            Err("native engine omitted a requested anchor".to_owned())
        } else {
            Ok(())
        }
    }
}

fn anchor_mark(anchor: &RequestedAnchor, resolution: AnchorResolution) -> Mark {
    Mark::Anchor(ResolvedAnchor {
        id: anchor.id.clone(),
        frame_offset: Some(0),
        resolution,
    })
}

fn words(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if !word_core(ch) {
            continue;
        }
        let mut end = start + ch.len_utf8();
        while let Some(&(position, ch)) = chars.peek() {
            if word_core(ch) {
                end = position + ch.len_utf8();
                chars.next();
            } else if matches!(ch, '\'' | '\u{2019}' | '-')
                && text[position + ch.len_utf8()..]
                    .chars()
                    .next()
                    .is_some_and(word_core)
            {
                chars.next();
            } else {
                break;
            }
        }
        spans.push((start, end));
        if spans.len() == MAX_MARKERS {
            break;
        }
    }
    spans
}
fn word_core(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn sentences(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, first)) = chars.next() {
        if first.is_whitespace() {
            continue;
        }
        let mut current = (start, first);
        let end = loop {
            let (position, ch) = current;
            if matches!(ch, '\r' | '\n') {
                break position;
            }
            let mut end = position + ch.len_utf8();
            if matches!(
                ch,
                '.' | '!' | '?' | '\u{2026}' | '\u{3002}' | '\u{ff01}' | '\u{ff1f}'
            ) {
                while let Some(&(position, ch)) = chars.peek() {
                    if !matches!(ch, '\'' | '"' | '\u{2019}' | '\u{201d}' | ')' | ']' | '}') {
                        break;
                    }
                    end = position + ch.len_utf8();
                    chars.next();
                }
                if chars.peek().is_none_or(|(_, ch)| ch.is_whitespace()) {
                    break end;
                }
            }
            match chars.next() {
                Some(next) => current = next,
                None => break text.len(),
            }
        };
        if end > start {
            spans.push((start, end));
        }
        if spans.len() == MAX_MARKERS {
            break;
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_spans_keep_utf8_offsets_and_sentence_punctuation() {
        let text = "Café can't stop.\nFile-name_2!";
        assert_eq!(
            words(text)
                .iter()
                .map(|&(s, e)| &text[s..e])
                .collect::<Vec<_>>(),
            ["Café", "can't", "stop", "File-name_2"]
        );
        assert_eq!(
            sentences(text)
                .iter()
                .map(|&(s, e)| &text[s..e])
                .collect::<Vec<_>>(),
            ["Café can't stop.", "File-name_2!"]
        );
    }
    #[test]
    fn exact_anchors_preserve_same_offset_order_and_reject_missing_or_duplicate_indexes() {
        let request = SynthesisRequest::new("Café next.", TtsSettings::default())
            .with_anchors(vec![
                RequestedAnchor::new("a", 6, AnchorAffinity::Before),
                RequestedAnchor::new("b", 6, AnchorAffinity::After),
            ])
            .unwrap();
        let mut plan = Plan::new(&request, true).unwrap();
        assert!(plan.finish().is_err());
        let mut reached = Vec::new();
        for (_, index) in plan.insertions.clone() {
            reached.extend(plan.reached(index, 123).unwrap());
        }
        assert!(plan.finish().is_ok());
        assert!(plan.reached(plan.insertions[0].1, 123).is_err());
        assert_eq!(
            reached
                .iter()
                .filter(|m| matches!(m, Mark::Anchor(_)))
                .count(),
            2
        );
        assert!(reached.iter().all(|m| m.frame() == 123));
    }
    #[test]
    fn dectalk_anchors_follow_windows_word_affinity_and_span_fallback() {
        let request = SynthesisRequest::new("one two", TtsSettings::default())
            .with_anchors(vec![
                RequestedAnchor::new("before", 2, AnchorAffinity::Before),
                RequestedAnchor::new("after", 2, AnchorAffinity::After),
                RequestedAnchor::new("outside", 7, AnchorAffinity::Before),
            ])
            .unwrap();
        let mut plan = Plan::new(&request, false).unwrap();
        assert!(
            matches!(&plan.leading[0], Mark::Anchor(m) if m.id == "outside" && m.resolution == AnchorResolution::SpanBoundary)
        );
        for (position, index) in plan.insertions.clone() {
            for mark in plan.reached(index, position as u64).unwrap() {
                if let Mark::Anchor(mark) = mark {
                    assert_eq!(
                        mark.frame_offset,
                        Some(if mark.id == "before" { 4 } else { 0 })
                    );
                    assert_eq!(mark.resolution, AnchorResolution::WordBoundary);
                }
            }
        }
        plan.finish().unwrap();
    }
}
