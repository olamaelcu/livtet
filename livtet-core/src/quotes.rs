//! Literary quotation pool used by the FFI surface to render time-of-day
//! greetings on the dashboard and empty-state fillers in library views.
//!
//! The corpus lives at `data/quotes/` inside this crate. `greetings/<period>.txt`
//! holds 4-line quote blocks (text / author / work / `===` separator) for each
//! of the six time-of-day periods; `timing/<period>.txt` holds short greeting
//! labels (one per line); `empty.txt` holds the period-agnostic pool used by
//! the empty-state filler.
//!
//! [`pick_greeting`] and [`pick_empty`] derive the period and hour from the
//! device's local clock (falling back to UTC if local offset can't be
//! determined) and pick a stable quote + label for that `(period, hour)`
//! bucket using a `blake3` hash. The hash makes the selection deterministic
//! for the duration of an hour and stable across builds, so the mobile UI
//! sees the same greeting until the wall clock rolls over.
//!
//! Test-only [`pick_greeting_at`] and [`pick_empty_at`] let callers pin both
//! inputs so the selection logic can be asserted without depending on the
//! clock.

use std::sync::OnceLock;

use time::OffsetDateTime;

// ── Period ─────────────────────────────────────────────────────────────────

/// Six time-of-day buckets the greeting pool is organised around. Each
/// bucket maps to a file under `data/quotes/greetings/<file_key>.txt` and a
/// label file under `data/quotes/timing/<file_key>.txt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Period {
    EarlyMorning,
    LateMorning,
    Afternoon,
    Evening,
    Night,
    LateNight,
}

impl Period {
    /// Bucket an hour of day (0-23) into one of the six periods. Boundaries:
    ///
    /// - `0..=3`  → [`Period::LateNight`]
    /// - `4..=8`  → [`Period::EarlyMorning`]
    /// - `9..=11` → [`Period::LateMorning`]
    /// - `12..=16`→ [`Period::Afternoon`]
    /// - `17..=19`→ [`Period::Evening`]
    /// - `20..=23`→ [`Period::Night`]
    pub fn from_hour(hour: u8) -> Self {
        match hour {
            0..=3 => Self::LateNight,
            4..=8 => Self::EarlyMorning,
            9..=11 => Self::LateMorning,
            12..=16 => Self::Afternoon,
            17..=19 => Self::Evening,
            _ => Self::Night,
        }
    }

    /// Stable identifier used as the on-disk filename and the hash domain
    /// separator (snake_case, matches `data/quotes/greetings/<file_key>.txt`).
    pub fn as_file_key(self) -> &'static str {
        match self {
            Self::EarlyMorning => "early_morning",
            Self::LateMorning => "late_morning",
            Self::Afternoon => "afternoon",
            Self::Evening => "evening",
            Self::Night => "night",
            Self::LateNight => "late_night",
        }
    }

    /// Human-readable label for the period, displayed in the greeting's
    /// `period` field and surfaced verbatim on the mobile dashboard.
    /// Matches the fixture values used by the iOS snapshot tests
    /// (`Early Morning`, `Late Morning`, ...).
    pub fn as_display_name(self) -> &'static str {
        match self {
            Self::EarlyMorning => "Early Morning",
            Self::LateMorning => "Late Morning",
            Self::Afternoon => "Afternoon",
            Self::Evening => "Evening",
            Self::Night => "Night",
            Self::LateNight => "Late Night",
        }
    }

    /// Stable discriminant index used to look up into the static pool
    /// arrays. Mirrors the order of variants in [`Period`].
    fn index(self) -> usize {
        match self {
            Self::EarlyMorning => 0,
            Self::LateMorning => 1,
            Self::Afternoon => 2,
            Self::Evening => 3,
            Self::Night => 4,
            Self::LateNight => 5,
        }
    }
}

// ── Public records ────────────────────────────────────────────────────────

/// A literary greeting drawn from African American and African diaspora
/// authors, chosen for the current time of day. Returned by
/// `livtet_ffi::get_greeting`.
#[derive(Debug, Clone)]
pub struct Greeting {
    /// Short conversational label like "Good morning" — used as the iOS
    /// navigation-bar title and the Android greeting chip.
    pub label: String,
    /// The literary quotation itself.
    pub text: String,
    pub author: String,
    pub material: String,
    /// Human-readable period name, e.g. "Early Morning".
    pub period: String,
}

/// An empty-state filler: a literary quotation without a time-of-day period
/// or greeting label. Returned by `livtet_ffi::get_empty_state_quotation`
/// and rendered into any list or view that has no rows.
#[derive(Debug, Clone)]
pub struct EmptyMessage {
    pub text: String,
    pub author: String,
    pub material: String,
}

// ── Internals ─────────────────────────────────────────────────────────────

struct Quote {
    text: &'static str,
    author: &'static str,
    material: &'static str,
}

struct Pool {
    by_period: [Vec<Quote>; 6],
    labels_by_period: [Vec<&'static str>; 6],
    empty: Vec<Quote>,
}

static POOL: OnceLock<Pool> = OnceLock::new();

fn pool() -> &'static Pool {
    POOL.get_or_init(build_pool)
}

fn build_pool() -> Pool {
    let greeting_files = [
        include_str!("../data/quotes/greetings/early_morning.txt"),
        include_str!("../data/quotes/greetings/late_morning.txt"),
        include_str!("../data/quotes/greetings/afternoon.txt"),
        include_str!("../data/quotes/greetings/evening.txt"),
        include_str!("../data/quotes/greetings/night.txt"),
        include_str!("../data/quotes/greetings/late_night.txt"),
    ];
    let label_files = [
        include_str!("../data/quotes/timing/early_morning.txt"),
        include_str!("../data/quotes/timing/late_morning.txt"),
        include_str!("../data/quotes/timing/afternoon.txt"),
        include_str!("../data/quotes/timing/evening.txt"),
        include_str!("../data/quotes/timing/night.txt"),
        include_str!("../data/quotes/timing/late_night.txt"),
    ];

    let mut by_period: [Vec<Quote>; 6] = std::array::from_fn(|_| Vec::new());
    for (i, content) in greeting_files.into_iter().enumerate() {
        by_period[i] = parse_blocks(content);
    }
    let mut labels_by_period: [Vec<&'static str>; 6] =
        std::array::from_fn(|_| Vec::new());
    for (i, content) in label_files.into_iter().enumerate() {
        labels_by_period[i] = parse_labels(content);
    }
    let empty = parse_blocks(include_str!("../data/quotes/empty.txt"));

    Pool {
        by_period,
        labels_by_period,
        empty,
    }
}

/// Split a quote file into 4-line blocks. The separator is a line whose
/// only non-whitespace content is `===`. Each block is `text / author / work`.
/// Blocks that don't have exactly 3 non-blank lines are silently skipped.
///
/// Requires `&'static str` because the resulting `Quote` references are
/// stored in the static pool.
fn parse_blocks(content: &'static str) -> Vec<Quote> {
    let mut out = Vec::new();
    for block in content.split("\n===\n") {
        let trimmed = block.trim_matches('\n');
        let mut lines = trimmed.lines().map(str::trim);
        let text = lines.next().unwrap_or("");
        let author = lines.next().unwrap_or("");
        let material = lines.next().unwrap_or("");
        if text.is_empty() || author.is_empty() || material.is_empty() {
            continue;
        }
        // Anything past the first three lines is treated as part of the
        // material (so a quote with internal newlines collapses gracefully
        // onto one line).
        let material_end: String = std::iter::once(material)
            .chain(lines.filter(|l| !l.is_empty()))
            .collect::<Vec<_>>()
            .join(" ");
        out.push(Quote {
            text,
            author,
            material: leak_str(&material_end),
        });
    }
    out
}

/// Split a label file on newlines, trim, drop blanks. Labels are short
/// strings like "Good afternoon".
///
/// Requires `&'static str` because the resulting slice references are
/// stored in the static pool.
fn parse_labels(content: &'static str) -> Vec<&'static str> {
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect()
}

/// Promote a `String` to a `&'static str` by leaking. Used only at static
/// initialisation time (inside `OnceLock`), so the leak is bounded to one
/// allocation per material-with-trailing-text quote block.
fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

/// Hash `(period, hour)` with `blake3` and reduce to a `usize` for index
/// selection. `blake3` is already a dependency of this crate.
fn hash(period: Period, hour: u8) -> usize {
    let mut hasher = blake3::Hasher::new();
    hasher.update(period.as_file_key().as_bytes());
    hasher.update(&[hour]);
    let bytes: [u8; 8] = hasher.finalize().as_bytes()[..8]
        .try_into()
        .expect("blake3 output is 32 bytes");
    u64::from_le_bytes(bytes) as usize
}

/// Same shape as [`hash`] but for the period-agnostic empty-state pool.
/// Distinct domain separator avoids any accidental alignment with the
/// per-period hash buckets.
fn hash_empty(hour: u8) -> usize {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"empty");
    hasher.update(&[hour]);
    let bytes: [u8; 8] = hasher.finalize().as_bytes()[..8]
        .try_into()
        .expect("blake3 output is 32 bytes");
    u64::from_le_bytes(bytes) as usize
}

/// Read the local clock and return `(period, hour)`. Falls back to UTC if
/// the local offset can't be determined (e.g. inside a container with no
/// `/etc/localtime`).
fn now_local_period_hour() -> (Period, u8) {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    (Period::from_hour(now.hour()), now.hour())
}

// ── Public selectors ─────────────────────────────────────────────────────

/// Pick a greeting for the current time. Reads the local clock and hashes
/// `(period, hour)` to choose a stable quote + label for the duration of
/// that hour. Use [`pick_greeting_at`] in tests to pin both inputs.
pub fn pick_greeting() -> Greeting {
    let (period, hour) = now_local_period_hour();
    pick_greeting_at(period, hour)
}

/// Pick an empty-state filler for the current hour. Reads the local clock
/// and hashes `("empty", hour)` to choose a stable quote. Use
/// [`pick_empty_at`] in tests.
pub fn pick_empty() -> EmptyMessage {
    let (_, hour) = now_local_period_hour();
    pick_empty_at(hour)
}

/// Test-only variant of [`pick_greeting`] that takes `period` and `hour`
/// directly. The same `(period, hour)` always produces the same output.
pub fn pick_greeting_at(period: Period, hour: u8) -> Greeting {
    let p = pool();
    let quotes = &p.by_period[period.index()];
    let labels = &p.labels_by_period[period.index()];
    debug_assert!(!quotes.is_empty(), "period pool must be non-empty");
    debug_assert!(!labels.is_empty(), "label pool must be non-empty");
    let idx = hash(period, hour) % quotes.len();
    let label_idx = hash(period, hour) % labels.len();
    let q = &quotes[idx];
    Greeting {
        label: labels[label_idx].to_string(),
        text: q.text.to_string(),
        author: q.author.to_string(),
        material: q.material.to_string(),
        period: period.as_display_name().to_string(),
    }
}

/// Test-only variant of [`pick_empty`] that takes `hour` directly.
pub fn pick_empty_at(hour: u8) -> EmptyMessage {
    let p = pool();
    debug_assert!(!p.empty.is_empty(), "empty pool must be non-empty");
    let idx = hash_empty(hour) % p.empty.len();
    let q = &p.empty[idx];
    EmptyMessage {
        text: q.text.to_string(),
        author: q.author.to_string(),
        material: q.material.to_string(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_hour_buckets_correctly() {
        assert_eq!(Period::from_hour(0), Period::LateNight);
        assert_eq!(Period::from_hour(3), Period::LateNight);
        assert_eq!(Period::from_hour(4), Period::EarlyMorning);
        assert_eq!(Period::from_hour(8), Period::EarlyMorning);
        assert_eq!(Period::from_hour(9), Period::LateMorning);
        assert_eq!(Period::from_hour(11), Period::LateMorning);
        assert_eq!(Period::from_hour(12), Period::Afternoon);
        assert_eq!(Period::from_hour(16), Period::Afternoon);
        assert_eq!(Period::from_hour(17), Period::Evening);
        assert_eq!(Period::from_hour(19), Period::Evening);
        assert_eq!(Period::from_hour(20), Period::Night);
        assert_eq!(Period::from_hour(23), Period::Night);
    }

    #[test]
    fn display_names_have_spaces() {
        // The fixtures used by the iOS snapshot tests expect these exact
        // capitalised-with-space strings.
        assert_eq!(Period::EarlyMorning.as_display_name(), "Early Morning");
        assert_eq!(Period::LateMorning.as_display_name(), "Late Morning");
        assert_eq!(Period::Afternoon.as_display_name(), "Afternoon");
        assert_eq!(Period::Evening.as_display_name(), "Evening");
        assert_eq!(Period::Night.as_display_name(), "Night");
        assert_eq!(Period::LateNight.as_display_name(), "Late Night");
    }

    #[test]
    fn file_keys_are_snake_case() {
        // Used as both the on-disk filename and the hash domain separator.
        assert_eq!(Period::EarlyMorning.as_file_key(), "early_morning");
        assert_eq!(Period::LateNight.as_file_key(), "late_night");
    }

    #[test]
    fn parse_blocks_splits_on_separator() {
        let content =
            "first text\nfirst author\nfirst work\n===\nsecond text\nsecond author\nsecond work\n";
        let quotes = parse_blocks(content);
        assert_eq!(quotes.len(), 2);
        assert_eq!(quotes[0].text, "first text");
        assert_eq!(quotes[0].author, "first author");
        assert_eq!(quotes[0].material, "first work");
        assert_eq!(quotes[1].text, "second text");
    }

    #[test]
    fn parse_blocks_skips_malformed_blocks() {
        // Only one line in the first block — must be skipped.
        let content =
            "lonely line\n===\nfull text\nfull author\nfull work\n";
        let quotes = parse_blocks(content);
        assert_eq!(quotes.len(), 1);
        assert_eq!(quotes[0].text, "full text");
    }

    #[test]
    fn parse_blocks_trims_whitespace() {
        let content = "  text \n  author \n  work \n===\n";
        let quotes = parse_blocks(content);
        assert_eq!(quotes.len(), 1);
        assert_eq!(quotes[0].text, "text");
        assert_eq!(quotes[0].author, "author");
        assert_eq!(quotes[0].material, "work");
    }

    #[test]
    fn parse_labels_skips_blanks() {
        let content = "Good morning\n\nEarly start\n   \nWhere was I in that book?\n";
        let labels = parse_labels(content);
        assert_eq!(
            labels,
            vec!["Good morning", "Early start", "Where was I in that book?"]
        );
    }

    #[test]
    fn pick_greeting_at_is_deterministic() {
        let a = pick_greeting_at(Period::Afternoon, 14);
        let b = pick_greeting_at(Period::Afternoon, 14);
        assert_eq!(a.text, b.text);
        assert_eq!(a.author, b.author);
        assert_eq!(a.material, b.material);
        assert_eq!(a.label, b.label);
        assert_eq!(a.period, b.period);
    }

    #[test]
    fn pick_greeting_at_populates_all_fields() {
        let g = pick_greeting_at(Period::Evening, 19);
        assert!(!g.text.is_empty());
        assert!(!g.author.is_empty());
        assert!(!g.material.is_empty());
        assert!(!g.label.is_empty());
        assert_eq!(g.period, "Evening");
    }

    #[test]
    fn pick_greeting_at_changes_with_hour() {
        // Pool sizes are >= 12 per period, so a hash collision between two
        // hours is vanishingly unlikely. Assert inequality; if it ever
        // fails the test reveals either a hash collision (raise pool size)
        // or a wiring bug (the same index was returned for two distinct
        // inputs).
        let g4 = pick_greeting_at(Period::EarlyMorning, 4);
        let g8 = pick_greeting_at(Period::EarlyMorning, 8);
        assert_ne!(
            (g4.text.clone(), g4.author.clone()),
            (g8.text.clone(), g8.author.clone()),
            "different hours should produce different quotes (with very high probability)",
        );
    }

    #[test]
    fn pick_empty_at_is_deterministic() {
        let a = pick_empty_at(10);
        let b = pick_empty_at(10);
        assert_eq!(a.text, b.text);
        assert_eq!(a.author, b.author);
        assert_eq!(a.material, b.material);
    }

    #[test]
    fn pick_empty_at_populates_all_fields() {
        let e = pick_empty_at(10);
        assert!(!e.text.is_empty());
        assert!(!e.author.is_empty());
        assert!(!e.material.is_empty());
    }

    #[test]
    fn pick_greeting_no_args_returns_populated() {
        // Uses the real clock — only asserts the fields are non-empty,
        // not specific values, so it stays robust across time zones.
        let g = pick_greeting();
        assert!(!g.text.is_empty(), "text");
        assert!(!g.author.is_empty(), "author");
        assert!(!g.material.is_empty(), "material");
        assert!(!g.label.is_empty(), "label");
        assert!(!g.period.is_empty(), "period");
    }

    #[test]
    fn pick_empty_no_args_returns_populated() {
        let e = pick_empty();
        assert!(!e.text.is_empty(), "text");
        assert!(!e.author.is_empty(), "author");
        assert!(!e.material.is_empty(), "material");
    }

    #[test]
    fn pool_loads_at_least_twelve_quotes_per_period() {
        // Sanity check: the bundled pools should each be substantial enough
        // to keep `(period, hour)` hash collisions rare.
        let p = pool();
        for (period, vec) in p.by_period.iter().enumerate() {
            assert!(
                vec.len() >= 12,
                "period {period} pool has only {} quotes; expected >= 12",
                vec.len()
            );
        }
        assert!(p.empty.len() >= 12, "empty pool has only {} quotes", p.empty.len());
    }
}
