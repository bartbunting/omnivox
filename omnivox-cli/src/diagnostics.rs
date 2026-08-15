//! Explicitly opt-in diagnostics that may contain private speech content.

pub const SYNTHESIS_TEXT_LOG_ENV: &str = "OMNIVOX_LOG_SYNTHESIS_TEXT";

pub fn synthesis_text_logging_enabled() -> bool {
    std::env::var(SYNTHESIS_TEXT_LOG_ENV).is_ok_and(|value| opt_in_value(&value))
}

fn opt_in_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::opt_in_value;

    #[test]
    fn synthesis_text_logging_requires_an_explicit_true_value() {
        for value in ["1", "true", "TRUE", " yes ", "On"] {
            assert!(opt_in_value(value), "{value:?} should enable text logging");
        }
        for value in ["", "0", "false", "no", "enabled", "garbage"] {
            assert!(
                !opt_in_value(value),
                "{value:?} should not enable text logging"
            );
        }
    }
}
