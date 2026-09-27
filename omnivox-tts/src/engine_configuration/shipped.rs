//! Canonical identities are reserved even when a platform or feature is absent.

pub const ALIASES: &[&str] = &["native"];

pub struct ShippedEngine {
    pub id: &'static str,
    pub in_process: bool,
    pub helper_environment: Option<&'static str>,
}

pub const ENGINES: &[ShippedEngine] = &[
    ShippedEngine {
        id: "espeak",
        in_process: true,
        helper_environment: None,
    },
    ShippedEngine {
        id: "winrt",
        in_process: true,
        helper_environment: None,
    },
    ShippedEngine {
        id: "macos",
        in_process: true,
        helper_environment: None,
    },
    ShippedEngine {
        id: "piper",
        in_process: false,
        helper_environment: Some("OMNIVOX_PIPER_HELPER"),
    },
    ShippedEngine {
        id: "rhvoice",
        in_process: false,
        helper_environment: Some("OMNIVOX_RHVOICE_HELPER"),
    },
    ShippedEngine {
        id: "flite",
        in_process: false,
        helper_environment: Some("OMNIVOX_FLITE_HELPER"),
    },
    ShippedEngine {
        id: "rutts",
        in_process: false,
        helper_environment: Some("OMNIVOX_RUTTS_HELPER"),
    },
    ShippedEngine {
        id: "tgspeechbox",
        in_process: false,
        helper_environment: Some("OMNIVOX_TGSPEECHBOX_HELPER"),
    },
    ShippedEngine {
        id: "eloquence",
        in_process: false,
        helper_environment: Some("OMNIVOX_ELOQUENCE_HELPER"),
    },
    ShippedEngine {
        id: "dectalk",
        in_process: false,
        helper_environment: Some("OMNIVOX_DECTALK_HELPER"),
    },
    ShippedEngine {
        id: "mbrola",
        in_process: false,
        helper_environment: Some("OMNIVOX_MBROLA_HELPER"),
    },
];

pub fn definition(id: &str) -> Option<&'static ShippedEngine> {
    ENGINES.iter().find(|engine| engine.id == id)
}

pub fn reserved(id: &str) -> bool {
    definition(id).is_some() || ALIASES.contains(&id)
}
