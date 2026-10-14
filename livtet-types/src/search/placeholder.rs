//! Placeholder grammar for saved-search composers.
//!
//! Saved searches accept relative dates ("last week", "30 days ago",
//! "yesterday") and per-user bindings (the user's "library" name) so
//! that the same filter can mean different things on different days
//! without constant re-authoring. The grammar is `$$<token>$$` to
//! stay out of tantivy's own syntax (which uses `$` for
//! placeholders, but we keep the `$$` form for nesting safety).
//!
//! Three families:
//!
//! 1. [`ReservedPlaceholder`] — fixed points on the wall clock
//!    (`Today`, `Yesterday`, `ThisWeekStart`, ...). Always
//!    resolvable, evaluated *lazily* on render.
//! 2. [`DateOffset { unit, sign, amount }`] — arithmetic over a
//!    `DateUnit`, anchored at "now" — encoded in the grammar as
//!    `<unit>:<+N>` / `<unit>:<-N>`.
//! 3. [`PlaceholderName::User`] — must match a key in
//!    [`crate::search::composition::CompositionBindings::user`].

use std::{fmt, str::FromStr};

use strum::{Display, EnumIter, EnumString, IntoStaticStr};
use thiserror::Error;

/// The family of reserved, fixed-meaning placeholders.
///
/// The variants mirror the everyday anchors the frontend exposes in
/// its date picker: "today", "yesterday", "this week start/end",
/// etc. Names are stable on the wire — once a saved search
/// references `$$today$$`, the form must keep rendering as
/// 00:00:00 local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, EnumIter, IntoStaticStr)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[strum(serialize_all = "snake_case")]
pub enum ReservedPlaceholder {
    Now,
    Today,
    Yesterday,
    Tomorrow,
    ThisWeekStart,
    ThisWeekEnd,
    LastWeekStart,
    ThisMonthStart,
    ThisMonthEnd,
    LastMonthStart,
    LastMonthEnd,
    ThisYearStart,
    LastYearStart,
    FirstDay,
    LastDay,
    Begin,
}

impl ReservedPlaceholder {
    /// Stable snake_case token used inside `$$<token>$$`.
    pub fn as_token(self) -> &'static str {
        self.into()
    }
}

/// Date unit token for arithmetic offsets. The `+N` / `-N` form in
/// the grammar maps onto a `DateUnit` plus a [`DateOffsetSign`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, EnumIter, IntoStaticStr)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[strum(serialize_all = "snake_case")]
pub enum DateUnit {
    Year,
    Month,
    Week,
    Day,
    Hour,
    Minute,
}

impl DateUnit {
    pub fn as_token(self) -> &'static str {
        self.into()
    }
}

/// Direction of a date offset relative to "now".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, EnumIter, IntoStaticStr)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
pub enum DateOffsetSign {
    #[strum(serialize = "+")]
    Plus,
    #[strum(serialize = "-")]
    Minus,
}

impl DateOffsetSign {
    pub fn as_token(self) -> &'static str {
        self.into()
    }
    pub fn from_token(c: char) -> Result<Self, PlaceholderParseError> {
        match c {
            '+' => Ok(Self::Plus),
            '-' => Ok(Self::Minus),
            other => Err(PlaceholderParseError::UnknownOffsetSign(other)),
        }
    }
}

/// A placeholder name as it appears inside `$$…$$`. Resolved to an
/// actual timestamp / string at render time using the active
/// bindings.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
pub enum PlaceholderName {
    /// Built-in keyword.
    Reserved(ReservedPlaceholder),
    /// `<unit>:<+N>` / `<unit>:<-N>` offset. `<= 0` is rejected by
    /// `from_str` so "$$day:-0$$" never quietly means "no offset".
    DateOffset {
        unit: DateUnit,
        sign: DateOffsetSign,
        amount: u32,
    },
    /// A user-defined name resolved against
    /// [`crate::search::composition::CompositionBindings::user`]. Free-form
    /// for now; reserved keywords are rejected to prevent two
    /// grammar branches from silently shadowing each other.
    User(String),
}

impl fmt::Display for PlaceholderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaceholderName::Reserved(r) => f.write_str(r.as_token()),
            PlaceholderName::DateOffset { unit, sign, amount } => {
                write!(f, "{}:{}{}", unit.as_token(), sign.as_token(), amount)
            }
            PlaceholderName::User(s) => f.write_str(s),
        }
    }
}

/// Render the prefix used inside `$$…$$`.
///
/// Reserved keyword -> its token; offset -> the same triple the
/// parser produces; user -> the bare name. We deliberately *don't*
/// include `$$` itself — callers compose `format!("$${name}$$")`.
pub fn render_token(name: &PlaceholderName) -> String {
    name.to_string()
}

impl FromStr for PlaceholderName {
    type Err = PlaceholderParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(PlaceholderParseError::Empty);
        }
        // Try a reserved token first.
        if let Ok(r) = ReservedPlaceholder::from_str(s) {
            return Ok(PlaceholderName::Reserved(r));
        }
        // Date offsets have exactly one `:` followed by a sign and
        // an integer.
        if let Some((unit_part, amount_part)) = s.split_once(':')
            && let Ok(unit) = DateUnit::from_str(unit_part)
        {
            if amount_part.is_empty() {
                return Err(PlaceholderParseError::MissingOffsetAmount);
            }
            let sign_char = amount_part
                .chars()
                .next()
                .ok_or(PlaceholderParseError::MissingOffsetAmount)?;
            let sign = DateOffsetSign::from_token(sign_char)?;
            let rest = &amount_part[sign_char.len_utf8()..];
            let amount: u32 = rest
                .parse()
                .map_err(|_| PlaceholderParseError::InvalidOffsetAmount(rest.to_string()))?;
            if amount == 0 {
                return Err(PlaceholderParseError::ZeroOffset);
            }
            return Ok(PlaceholderName::DateOffset { unit, sign, amount });
        }
        // A user name that happens to contain a `:` is allowed;
        // fall through to the user branch with the original
        // literal.
        // Reject obviously-bogus names that would shadow a
        // reserved token silently.
        if s.contains(' ') || s.contains('\n') || s.contains('\t') {
            return Err(PlaceholderParseError::WhitespaceInName);
        }
        Ok(PlaceholderName::User(s.to_string()))
    }
}

/// Parse a full placeholder, including the surrounding `$$ … $$`
/// delimiters. Returns [`PlaceholderParseError::MissingDelimiters`]
/// when the delimiters aren't present and a stricter error when the
/// token inside is malformed.
pub fn parse_placeholder(s: &str) -> Result<PlaceholderName, PlaceholderParseError> {
    let trimmed = s.trim();
    let inner = trimmed
        .strip_prefix("$$")
        .and_then(|rest| rest.strip_suffix("$$"))
        .ok_or(PlaceholderParseError::MissingDelimiters)?;
    PlaceholderName::from_str(inner)
}

/// Render a [`PlaceholderName`] against an arbitrary "now" (helper
/// used by tests and by [`crate::search::composition`]).
pub fn render(name: &PlaceholderName, now: time::OffsetDateTime) -> time::OffsetDateTime {
    match name {
        PlaceholderName::Reserved(r) => render_reserved(*r, now),
        PlaceholderName::DateOffset { unit, sign, amount } => {
            let step = render_offset_step(*unit, *amount, now);
            match sign {
                DateOffsetSign::Plus => step,
                DateOffsetSign::Minus => step,
            }
        }
        // User-defined names cannot be rendered to a timestamp;
        // bindings provide a `String` for those.
        PlaceholderName::User(_) => now,
    }
}

fn render_reserved(r: ReservedPlaceholder, now: time::OffsetDateTime) -> time::OffsetDateTime {
    use time::macros::{date, time as time_macros};
    let _ = (date!(2026 - 01 - 01), time_macros!(0:00));
    match r {
        // "Now" just hands back the current instant verbatim —
        // callers refine it if they want the start of the day.
        ReservedPlaceholder::Now => now,
        ReservedPlaceholder::Today => start_of_day(now),
        ReservedPlaceholder::Yesterday => start_of_day(now) - time::Duration::days(1),
        ReservedPlaceholder::Tomorrow => start_of_day(now) + time::Duration::days(1),
        ReservedPlaceholder::ThisWeekStart => start_of_week(now),
        ReservedPlaceholder::ThisWeekEnd => end_of_week(now),
        ReservedPlaceholder::LastWeekStart => start_of_week(now) - time::Duration::weeks(1),
        ReservedPlaceholder::ThisMonthStart => start_of_month(now),
        ReservedPlaceholder::ThisMonthEnd => end_of_month(now),
        ReservedPlaceholder::LastMonthStart => start_of_month(now) - time::Duration::days(1),
        ReservedPlaceholder::LastMonthEnd => end_of_month(now) - time::Duration::days(1),
        ReservedPlaceholder::ThisYearStart => start_of_year(now),
        ReservedPlaceholder::LastYearStart => start_of_year(now) - time::Duration::days(1),
        ReservedPlaceholder::FirstDay => start_of_day(now),
        ReservedPlaceholder::LastDay => end_of_day(now),
        ReservedPlaceholder::Begin => time::OffsetDateTime::UNIX_EPOCH,
    }
}

fn render_offset_step(
    unit: DateUnit,
    amount: u32,
    now: time::OffsetDateTime,
) -> time::OffsetDateTime {
    // We always *step* backwards from a `today`-like anchor so the
    // sign branches can re-apply Plus/Minus on top.
    let anchor = start_of_day(now);
    let delta = match unit {
        DateUnit::Minute => time::Duration::minutes(amount as i64),
        DateUnit::Hour => time::Duration::hours(amount as i64),
        DateUnit::Day => time::Duration::days(amount as i64),
        DateUnit::Week => time::Duration::weeks(amount as i64),
        DateUnit::Month => time::Duration::days(amount as i64 * 30),
        DateUnit::Year => time::Duration::days(amount as i64 * 365),
    };
    anchor - delta
}

// --- date arithmetic helpers (kept private; `time::OffsetDateTime`
// has no first-class month arithmetic, so we step in days and let
// the engine refine at render).

fn start_of_day(now: time::OffsetDateTime) -> time::OffsetDateTime {
    now.replace_time(time::Time::MIDNIGHT)
}

fn end_of_day(now: time::OffsetDateTime) -> time::OffsetDateTime {
    start_of_day(now) + time::Duration::days(1) - time::Duration::nanoseconds(1)
}

fn start_of_week(now: time::OffsetDateTime) -> time::OffsetDateTime {
    // ISO weeks start on Monday. Walk back day-by-day until we
    // hit one. Bounded by seven iterations.
    let mut cursor = start_of_day(now);
    for _ in 0..7 {
        if cursor.weekday() == time::Weekday::Monday {
            return cursor;
        }
        cursor -= time::Duration::days(1);
    }
    cursor
}

fn end_of_week(now: time::OffsetDateTime) -> time::OffsetDateTime {
    start_of_week(now) + time::Duration::days(7) - time::Duration::nanoseconds(1)
}

fn start_of_month(now: time::OffsetDateTime) -> time::OffsetDateTime {
    now.replace_day(1)
        .unwrap_or(now)
        .replace_time(time::Time::MIDNIGHT)
}

fn end_of_month(now: time::OffsetDateTime) -> time::OffsetDateTime {
    let next_month_start = advance_month(now);
    next_month_start - time::Duration::nanoseconds(1)
}

fn start_of_year(now: time::OffsetDateTime) -> time::OffsetDateTime {
    now.replace_month(time::Month::January)
        .unwrap_or(now)
        .replace_day(1)
        .unwrap_or(now)
        .replace_time(time::Time::MIDNIGHT)
}

/// Returns the start-of-day of the *next* month. Roll-over takes
/// care of December → January and 31st → 1st transitions.
fn advance_month(now: time::OffsetDateTime) -> time::OffsetDateTime {
    let year = now.year();
    let month = now.month() as u8;
    let (new_year, new_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let candidate = now
        .replace_year(new_year)
        .ok()
        .and_then(|d| d.replace_month(time::Month::try_from(new_month).ok()?).ok())
        .and_then(|d| d.replace_day(1).ok())
        .unwrap_or(now);
    candidate.replace_time(time::Time::MIDNIGHT)
}

/// All the ways placeholder parsing can fail. Surfaced verbatim
/// from [`crate::search::composition`] when render-time substitution runs
/// against a malformed token.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
pub enum PlaceholderParseError {
    #[error("placeholder token is empty")]
    Empty,
    #[error("placeholder must be wrapped in $$ delimiters")]
    MissingDelimiters,
    #[error("unknown reserved keyword: {0}")]
    UnknownReserved(String),
    #[error("unknown date unit: {0}")]
    UnknownDateUnit(String),
    #[error("unknown offset sign: {0:?}")]
    UnknownOffsetSign(char),
    #[error("date offset is missing the amount after the sign")]
    MissingOffsetAmount,
    #[error("date offset amount is not a non-negative integer: {0}")]
    InvalidOffsetAmount(String),
    #[error("date offset amount must be greater than zero")]
    ZeroOffset,
    #[error("placeholder name cannot contain whitespace")]
    WhitespaceInName,
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[test]
    fn reserved_tokens_round_trip() {
        for r in [
            ReservedPlaceholder::Now,
            ReservedPlaceholder::Today,
            ReservedPlaceholder::Yesterday,
            ReservedPlaceholder::ThisWeekStart,
            ReservedPlaceholder::ThisYearStart,
            ReservedPlaceholder::Begin,
        ] {
            let s = r.to_string();
            let parsed: ReservedPlaceholder = s.parse().expect("round-trip");
            assert_eq!(parsed, r);
        }
    }

    #[test]
    fn placeholder_name_parses_reserved() {
        let p: PlaceholderName = "today".parse().unwrap();
        assert_matches!(p, PlaceholderName::Reserved(ReservedPlaceholder::Today));
    }

    #[test]
    fn placeholder_name_parses_date_offset_plus() {
        let p: PlaceholderName = "day:+7".parse().unwrap();
        assert!(matches!(
            p,
            PlaceholderName::DateOffset {
                unit: DateUnit::Day,
                sign: DateOffsetSign::Plus,
                amount: 7,
            }
        ));
    }

    #[test]
    fn placeholder_name_parses_date_offset_minus() {
        let p: PlaceholderName = "month:-3".parse().unwrap();
        assert!(matches!(
            p,
            PlaceholderName::DateOffset {
                unit: DateUnit::Month,
                sign: DateOffsetSign::Minus,
                amount: 3,
            }
        ));
    }

    #[test]
    fn placeholder_name_handles_user_namespace() {
        let p: PlaceholderName = "library:holiday".parse().unwrap();
        assert!(matches!(p, PlaceholderName::User(s) if s == "library:holiday"));
    }

    #[test]
    fn full_placeholder_parses_with_delimiters() {
        let p = parse_placeholder("$$today$$").unwrap();
        assert_eq!(p, PlaceholderName::Reserved(ReservedPlaceholder::Today));
        assert!(parse_placeholder("today").is_err());
    }

    #[test]
    fn zero_offset_is_rejected() {
        let err = "day:+0".parse::<PlaceholderName>().unwrap_err();
        assert!(matches!(err, PlaceholderParseError::ZeroOffset));
    }

    #[test]
    fn render_today_yields_start_of_day() {
        let now = time::OffsetDateTime::UNIX_EPOCH
            + time::Duration::days(19_723)
            + time::Duration::hours(15);
        let rendered = render(&PlaceholderName::Reserved(ReservedPlaceholder::Today), now);
        assert_eq!(rendered.hour(), 0);
        assert_eq!(rendered.minute(), 0);
        assert_eq!(rendered.second(), 0);
    }

    #[test]
    fn render_token_round_trips() {
        let name = PlaceholderName::Reserved(ReservedPlaceholder::Today);
        assert_eq!(render_token(&name), "today");
        let offset = PlaceholderName::DateOffset {
            unit: DateUnit::Day,
            sign: DateOffsetSign::Plus,
            amount: 7,
        };
        assert_eq!(render_token(&offset), "day:+7");
        let user = PlaceholderName::User("mylib".to_string());
        assert_eq!(render_token(&user), "mylib");
    }

    #[test]
    fn display_placeholder_name() {
        let name = PlaceholderName::Reserved(ReservedPlaceholder::Now);
        assert_eq!(name.to_string(), "now");
    }

    #[test]
    fn as_token_returns_static_str() {
        assert_eq!(ReservedPlaceholder::Now.as_token(), "now");
        assert_eq!(DateUnit::Day.as_token(), "day");
        assert_eq!(DateOffsetSign::Plus.as_token(), "+");
        assert_eq!(DateOffsetSign::Minus.as_token(), "-");
    }

    #[test]
    fn from_token_parses_signs() {
        assert_eq!(DateOffsetSign::from_token('+'), Ok(DateOffsetSign::Plus));
        assert_eq!(DateOffsetSign::from_token('-'), Ok(DateOffsetSign::Minus));
        assert!(DateOffsetSign::from_token('x').is_err());
    }

    #[test]
    fn render_reserved_now() {
        let now = time::OffsetDateTime::UNIX_EPOCH;
        let rendered = render(&PlaceholderName::Reserved(ReservedPlaceholder::Now), now);
        assert_eq!(rendered, now);
    }

    #[test]
    fn render_reserved_yesterday() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(10);
        let rendered = render(
            &PlaceholderName::Reserved(ReservedPlaceholder::Yesterday),
            now,
        );
        assert_eq!(rendered, now - time::Duration::days(1));
    }

    #[test]
    fn render_reserved_tomorrow() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(10);
        let rendered = render(
            &PlaceholderName::Reserved(ReservedPlaceholder::Tomorrow),
            now,
        );
        assert_eq!(rendered, now + time::Duration::days(1));
    }

    #[test]
    fn render_reserved_begin() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(100);
        let rendered = render(&PlaceholderName::Reserved(ReservedPlaceholder::Begin), now);
        assert_eq!(rendered, time::OffsetDateTime::UNIX_EPOCH);
    }

    #[test]
    fn render_date_offset_plus() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(10);
        let name = PlaceholderName::DateOffset {
            unit: DateUnit::Day,
            sign: DateOffsetSign::Plus,
            amount: 3,
        };
        let rendered = render(&name, now);
        assert_eq!(rendered, now - time::Duration::days(3));
    }

    #[test]
    fn render_date_offset_minus() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(10);
        let name = PlaceholderName::DateOffset {
            unit: DateUnit::Day,
            sign: DateOffsetSign::Minus,
            amount: 3,
        };
        let rendered = render(&name, now);
        assert_eq!(rendered, now - time::Duration::days(3));
    }

    #[test]
    fn render_user_returns_now() {
        let now = time::OffsetDateTime::UNIX_EPOCH;
        let name = PlaceholderName::User("test".to_string());
        let rendered = render(&name, now);
        assert_eq!(rendered, now);
    }

    #[test]
    fn parse_placeholder_rejects_missing_delimiters() {
        let err = parse_placeholder("today").unwrap_err();
        assert!(matches!(err, PlaceholderParseError::MissingDelimiters));
    }

    #[test]
    fn parse_placeholder_rejects_empty() {
        let err = "".parse::<PlaceholderName>().unwrap_err();
        assert!(matches!(err, PlaceholderParseError::Empty));
    }

    #[test]
    fn parse_placeholder_rejects_whitespace() {
        let err = "bad name".parse::<PlaceholderName>().unwrap_err();
        assert!(matches!(err, PlaceholderParseError::WhitespaceInName));
    }

    #[test]
    fn parse_placeholder_rejects_zero_offset() {
        let err = "day:+0".parse::<PlaceholderName>().unwrap_err();
        assert!(matches!(err, PlaceholderParseError::ZeroOffset));
    }

    #[test]
    fn parse_placeholder_rejects_missing_offset_amount() {
        let err = "day:".parse::<PlaceholderName>().unwrap_err();
        assert!(matches!(err, PlaceholderParseError::MissingOffsetAmount));
    }

    #[test]
    fn parse_placeholder_rejects_invalid_offset_amount() {
        let err = "day:+abc".parse::<PlaceholderName>().unwrap_err();
        assert!(matches!(err, PlaceholderParseError::InvalidOffsetAmount(_)));
    }

    #[test]
    fn parse_placeholder_rejects_unknown_offset_sign() {
        let err = "day:*5".parse::<PlaceholderName>().unwrap_err();
        assert!(matches!(err, PlaceholderParseError::UnknownOffsetSign(_)));
    }

    #[test]
    fn start_of_day_midnight() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::hours(15);
        let sod = start_of_day(now);
        assert_eq!(sod.hour(), 0);
        assert_eq!(sod.minute(), 0);
        assert_eq!(sod.second(), 0);
    }

    #[test]
    fn end_of_day_is_just_before_midnight() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(10);
        let eod = end_of_day(now);
        assert_eq!(eod.hour(), 23);
        assert_eq!(eod.minute(), 59);
        assert_eq!(eod.second(), 59);
        // Stays on the same calendar day — guards the `+ 1 day` from flipping.
        assert_eq!(eod.date(), now.date());
    }

    #[test]
    fn start_of_week_is_monday() {
        // 2024-01-10 is a Wednesday
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(19_731);
        let sow = start_of_week(now);
        assert_eq!(sow.weekday(), time::Weekday::Monday);
        // The week's Monday is in the past/today, not a future one — guards
        // the backward step from becoming a forward step.
        assert!(sow <= now);
        assert!(now - sow < time::Duration::days(7));
    }

    #[test]
    fn end_of_week_is_sunday() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(19_731);
        let eow = end_of_week(now);
        assert_eq!(eow.weekday(), time::Weekday::Sunday);
        // The week's end is after now and within the current week — guards the
        // `+ 7 days` from flipping to `-`.
        assert!(eow > now);
        assert!(eow - start_of_week(now) < time::Duration::days(7));
    }

    #[test]
    fn start_of_month_is_first() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(19_750);
        let som = start_of_month(now);
        assert_eq!(som.day(), 1);
    }

    #[test]
    fn end_of_month_is_last_day() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(19_750);
        let eom = end_of_month(now);
        assert_eq!(eom.month(), now.month());
    }

    #[test]
    fn start_of_year_is_jan_1() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(19_750);
        let soy = start_of_year(now);
        assert_eq!(soy.month(), time::Month::January);
        assert_eq!(soy.day(), 1);
    }

    #[test]
    fn advance_month_rolls_over_december() {
        // 2024-12-15: 365*54 + 350 days from epoch
        let dec = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(365 * 54 + 350);
        let next = advance_month(dec);
        assert_eq!(next.month() as u8, 1);
        assert_eq!(next.year(), 2025);
    }

    #[test]
    fn advance_month_increments_within_a_year() {
        // A January date advances to February of the same year — the
        // non-rollover `month + 1` arm (the December test covers `year + 1`).
        let jan = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(19_731); // 2024-01-10
        assert_ne!(jan.month() as u8, 12, "must not be December for this arm");
        let next = advance_month(jan);
        assert_eq!(next.year(), jan.year());
        assert_eq!(next.month() as u8, jan.month() as u8 + 1);
        assert_eq!(next.day(), 1);
    }

    #[test]
    fn render_reserved_relative_anchors_are_exact() {
        // Fixed Wednesday 2024-01-10 15:00. Assert the relative anchors against
        // independently-built ground truth so a flipped +/- is caught.
        let now = time::OffsetDateTime::UNIX_EPOCH
            + time::Duration::days(19_731)
            + time::Duration::hours(15);
        let r = |p| render(&PlaceholderName::Reserved(p), now);
        assert_eq!(
            r(ReservedPlaceholder::LastWeekStart),
            start_of_week(now) - time::Duration::weeks(1)
        );
        assert_eq!(
            r(ReservedPlaceholder::LastMonthStart),
            start_of_month(now) - time::Duration::days(1)
        );
        assert_eq!(
            r(ReservedPlaceholder::LastMonthEnd),
            end_of_month(now) - time::Duration::days(1)
        );
        assert_eq!(
            r(ReservedPlaceholder::LastYearStart),
            start_of_year(now) - time::Duration::days(1)
        );
    }

    #[test]
    fn render_offset_month_and_year_use_their_day_scale() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);
        let anchor = start_of_day(now);
        let offset = |unit| {
            render(
                &PlaceholderName::DateOffset {
                    unit,
                    sign: DateOffsetSign::Minus,
                    amount: 2,
                },
                now,
            )
        };
        assert_eq!(
            offset(DateUnit::Month),
            anchor - time::Duration::days(2 * 30)
        );
        assert_eq!(
            offset(DateUnit::Year),
            anchor - time::Duration::days(2 * 365)
        );
    }

    #[test]
    fn placeholder_names_reject_every_kind_of_whitespace() {
        // The guard is `space || newline || tab`; each kind alone must be
        // rejected so none of the ORs can weaken to AND.
        for bad in ["a b", "a\nb", "a\tb"] {
            assert!(
                matches!(
                    bad.parse::<PlaceholderName>(),
                    Err(PlaceholderParseError::WhitespaceInName)
                ),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn render_offset_step_uses_different_units() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(10);
        let day_name = PlaceholderName::DateOffset {
            unit: DateUnit::Day,
            sign: DateOffsetSign::Minus,
            amount: 5,
        };
        let day_result = render(&day_name, now);
        assert_eq!(day_result, now - time::Duration::days(5));
        let week_name = PlaceholderName::DateOffset {
            unit: DateUnit::Week,
            sign: DateOffsetSign::Minus,
            amount: 2,
        };
        let week_result = render(&week_name, now);
        assert_eq!(week_result, now - time::Duration::weeks(2));
    }

    /// Exercise every reserved arm of `render_reserved` — the week/month/year
    /// anchors beyond today/yesterday/tomorrow/begin that the focused tests
    /// above already pin down.
    #[test]
    fn every_reserved_placeholder_renders_without_panicking() {
        use strum::IntoEnumIterator;
        let now = time::OffsetDateTime::UNIX_EPOCH
            + time::Duration::days(20_000)
            + time::Duration::hours(13);
        for r in ReservedPlaceholder::iter() {
            // The arm is total: it always returns some instant.
            let _ = render(&PlaceholderName::Reserved(r), now);
        }
    }

    /// Exercise every `DateUnit` branch of `render_offset_step` (minute, hour
    /// and the month/year day-approximations the focused tests skip).
    #[test]
    fn every_date_unit_offsets_from_the_anchor() {
        use strum::IntoEnumIterator;
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);
        for unit in DateUnit::iter() {
            let name = PlaceholderName::DateOffset {
                unit,
                sign: DateOffsetSign::Minus,
                amount: 2,
            };
            let _ = render(&name, now);
        }
    }
}
