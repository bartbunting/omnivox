use omnivox_tts::control::{ControlResponse, ControlResponseEnvelope};
use omnivox_tts::helper_protocol::{HelperRequest, HelperResponse};
use omnivox_tts::marker_protocol::{encode_marker_event, MarkerEventEnvelope};
use omnivox_tts::timeline_protocol::{
    validate_presentation_timeline, PresentationAction, PresentationTimelineEnvelope,
};

#[test]
fn documented_control_inventory_matches_wire_types() {
    let response: ControlResponseEnvelope = serde_json::from_str(include_str!(
        "../../docs/protocol-fixtures/control-inventory-response.json"
    ))
    .expect("documented control inventory must deserialize");

    assert!(matches!(
        response.response,
        ControlResponse::Inventory { .. }
    ));
}

#[test]
fn documented_timeline_matches_wire_types_and_semantic_validation() {
    let timeline: PresentationTimelineEnvelope = serde_json::from_str(include_str!(
        "../../docs/protocol-fixtures/presentation-timeline-v3.json"
    ))
    .expect("documented timeline must deserialize");

    validate_presentation_timeline(&timeline).expect("documented timeline must validate");
    assert!(timeline
        .actions
        .iter()
        .any(|action| matches!(action.action, PresentationAction::Audio { .. })));
    assert!(timeline
        .actions
        .iter()
        .any(|action| matches!(action.action, PresentationAction::Tone { .. })));
    assert!(timeline
        .actions
        .iter()
        .any(|action| matches!(action.action, PresentationAction::Silence { .. })));
    assert!(timeline
        .actions
        .iter()
        .any(|action| matches!(action.action, PresentationAction::SemanticEvent)));
}

#[test]
fn documented_helper_request_matches_wire_types_and_validation() {
    let request: HelperRequest = serde_json::from_str(include_str!(
        "../../docs/protocol-fixtures/helper-synthesize-request-v5.json"
    ))
    .expect("documented helper request must deserialize");

    request
        .validate()
        .expect("documented helper request must validate");
}

#[test]
fn documented_helper_success_stream_matches_wire_types_and_validation() {
    for line in include_str!("../../docs/protocol-fixtures/helper-synthesis-success-v5.jsonl")
        .lines()
        .filter(|line| !line.is_empty())
    {
        let response: HelperResponse =
            serde_json::from_str(line).expect("documented helper response must deserialize");
        response
            .validate()
            .expect("documented helper response must validate");
    }
}

#[test]
fn documented_marker_event_stream_matches_wire_types_and_validation() {
    for line in include_str!("../../docs/protocol-fixtures/playback-marker-events-v2.jsonl")
        .lines()
        .filter(|line| !line.is_empty())
    {
        let event: MarkerEventEnvelope =
            serde_json::from_str(line).expect("documented marker event must deserialize");
        encode_marker_event(&event).expect("documented marker event must validate");
    }
}
