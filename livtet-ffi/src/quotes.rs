//! Literary quotations for empty states and greetings.
//!
//! Thin re-export of the `livtet-temporal-quotes` corpus over FFI. The
//! shapes match what the mobile UI renders: [`Greeting`] carries the
//! time-of-day label, [`EmptyMessage`] intentionally does not.

use livtet_temporal_quotes as q;

/// A literary greeting chosen for the current time of day.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Greeting {
    /// Short conversational label, e.g. "Good morning".
    pub label: String,
    /// The literary quotation itself.
    pub text: String,
    pub author: String,
    pub material: String,
    /// Human-readable period name, e.g. "Early Morning".
    pub period: String,
}

/// An empty-state filler quotation: no time-of-day context.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EmptyMessage {
    pub text: String,
    pub author: String,
    pub material: String,
}

/// A greeting for right now. Infallible: the embedded corpus always
/// has an entry.
#[uniffi::export]
pub fn get_greeting() -> Greeting {
    let g = q::pick_greeting();
    Greeting {
        label: g.label,
        text: g.text,
        author: g.author,
        material: g.material,
        period: g.period,
    }
}

/// A quotation for empty surfaces (e.g. an empty library list).
#[uniffi::export]
pub fn get_empty_state_quotation() -> EmptyMessage {
    let e = q::pick_empty();
    EmptyMessage {
        text: e.text,
        author: e.author,
        material: e.material,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_greeting_populates_all_fields() {
        let g = get_greeting();
        assert!(!g.label.is_empty());
        assert!(!g.text.is_empty());
        assert!(!g.author.is_empty());
        assert!(!g.period.is_empty());
    }

    #[test]
    fn get_empty_state_quotation_populates_all_fields() {
        let m = get_empty_state_quotation();
        assert!(!m.text.is_empty());
        assert!(!m.author.is_empty());
    }
}
