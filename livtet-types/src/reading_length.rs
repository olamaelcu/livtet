use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, PartialEq, Eq, Type, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ReadingLength {
    Percentage,
    Pages(u32),
    VirtualPages(u32),
    Seconds(i64),
    Chapters(u32),
    Cfi,
}

impl ReadingLength {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        match self {
            Self::Percentage => {
                bytes.push(0);
            }
            Self::Pages(n) => {
                bytes.push(1);
                bytes.extend_from_slice(&n.to_le_bytes());
            }
            Self::VirtualPages(n) => {
                bytes.push(2);
                bytes.extend_from_slice(&n.to_le_bytes());
            }
            Self::Seconds(n) => {
                bytes.push(3);
                bytes.extend_from_slice(&n.to_le_bytes());
            }
            Self::Chapters(n) => {
                bytes.push(4);
                bytes.extend_from_slice(&n.to_le_bytes());
            }
            Self::Cfi => {
                bytes.push(5);
            }
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() {
            return None;
        }
        match bytes[0] {
            0 => Some(Self::Percentage),
            1 => {
                if bytes.len() >= 5 {
                    let n = u32::from_le_bytes(bytes[1..5].try_into().ok()?);
                    Some(Self::Pages(n))
                } else {
                    None
                }
            }
            2 => {
                if bytes.len() >= 5 {
                    let n = u32::from_le_bytes(bytes[1..5].try_into().ok()?);
                    Some(Self::VirtualPages(n))
                } else {
                    None
                }
            }
            3 => {
                if bytes.len() >= 9 {
                    let n = i64::from_le_bytes(bytes[1..9].try_into().ok()?);
                    Some(Self::Seconds(n))
                } else {
                    None
                }
            }
            4 => {
                if bytes.len() >= 5 {
                    let n = u32::from_le_bytes(bytes[1..5].try_into().ok()?);
                    Some(Self::Chapters(n))
                } else {
                    None
                }
            }
            5 => Some(Self::Cfi),
            _ => None,
        }
    }
}

pub fn progression_to_normalized(
    progression: &super::progress_unit::Progression,
    length: &ReadingLength,
) -> Option<f64> {
    use super::progress_unit::Progression;
    match (progression, length) {
        (Progression::Percentage(p), _) => Some(p.clamp(0.0, 1.0)),
        (Progression::Page(n), ReadingLength::Pages(c)) => {
            Some((*n as f64 / *c as f64).clamp(0.0, 1.0))
        }
        (Progression::VirtualPage(n), ReadingLength::VirtualPages(c)) => {
            Some((*n as f64 / *c as f64).clamp(0.0, 1.0))
        }
        (Progression::TimestampSeconds(s), ReadingLength::Seconds(c)) => {
            Some((*s as f64 / *c as f64).clamp(0.0, 1.0))
        }
        (Progression::Chapter(ch), ReadingLength::Chapters(c)) => {
            Some((*ch / *c as f64).clamp(0.0, 1.0))
        }
        (Progression::Cfi(_), _) => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::super::progress_unit::Progression;
    use super::{ReadingLength, progression_to_normalized};

    fn any_reading_length() -> impl Strategy<Value = ReadingLength> {
        prop_oneof![
            Just(ReadingLength::Percentage),
            any::<u32>().prop_map(ReadingLength::Pages),
            any::<u32>().prop_map(ReadingLength::VirtualPages),
            any::<i64>().prop_map(ReadingLength::Seconds),
            any::<u32>().prop_map(ReadingLength::Chapters),
            Just(ReadingLength::Cfi),
        ]
    }

    proptest! {
        /// The encoding is the stored form, so the round trip has to hold for
        /// every payload, not just the small ones -- `Seconds` carries a full
        /// i64 and the u32 variants reach the whole range.
        #[test]
        fn bytes_round_trip_for_every_variant(length in any_reading_length()) {
            let encoded = length.to_bytes();
            prop_assert_eq!(ReadingLength::from_bytes(&encoded), Some(length));
        }

        /// A discriminant the encoder never emits must decode to None rather
        /// than to some other variant.
        #[test]
        fn unknown_tags_decode_to_none(tag in 6u8..=255, tail in prop::collection::vec(any::<u8>(), 0..16)) {
            let mut bytes = vec![tag];
            bytes.extend_from_slice(&tail);
            prop_assert_eq!(ReadingLength::from_bytes(&bytes), None);
        }

        /// Every payload-carrying variant must refuse a truncated buffer
        /// instead of reading past the end or inventing a value.
        #[test]
        fn truncated_payloads_decode_to_none(length in any_reading_length()) {
            let encoded = length.to_bytes();
            if encoded.len() == 1 {
                return Ok(());  // Percentage and Cfi carry no payload.
            }
            for cut in 1..encoded.len() {
                prop_assert_eq!(
                    ReadingLength::from_bytes(&encoded[..cut]),
                    None,
                    "{:?} truncated to {} byte(s) must not decode", length, cut
                );
            }
        }

        /// Normalisation is a fraction of the whole, so it must stay in
        /// [0, 1] however far past the end the progress claims to be.
        #[test]
        fn normalized_progress_stays_in_the_unit_interval(
            position in 0u32..100_000,
            total in 1u32..100_000,
        ) {
            let got = progression_to_normalized(
                &Progression::Page(position),
                &ReadingLength::Pages(total),
            )
            .expect("page against a paged length");
            prop_assert!((0.0..=1.0).contains(&got), "got {got}");
        }
    }

    #[test]
    fn tags_are_stable_and_distinct() {
        // The first byte is the stored discriminant; changing one silently
        // reinterprets existing rows.
        for (length, tag) in [
            (ReadingLength::Percentage, 0u8),
            (ReadingLength::Pages(1), 1),
            (ReadingLength::VirtualPages(1), 2),
            (ReadingLength::Seconds(1), 3),
            (ReadingLength::Chapters(1), 4),
            (ReadingLength::Cfi, 5),
        ] {
            assert_eq!(length.to_bytes()[0], tag, "{length:?}");
        }
    }

    #[test]
    fn an_empty_buffer_decodes_to_none() {
        assert_eq!(ReadingLength::from_bytes(&[]), None);
    }

    #[test]
    fn normalization_pairs_each_progression_with_its_own_length() {
        // Matching pairs resolve.
        assert_eq!(
            progression_to_normalized(&Progression::Page(5), &ReadingLength::Pages(10)),
            Some(0.5)
        );
        assert_eq!(
            progression_to_normalized(
                &Progression::VirtualPage(1),
                &ReadingLength::VirtualPages(4)
            ),
            Some(0.25)
        );
        assert_eq!(
            progression_to_normalized(
                &Progression::TimestampSeconds(30),
                &ReadingLength::Seconds(120)
            ),
            Some(0.25)
        );
        assert_eq!(
            progression_to_normalized(&Progression::Chapter(3.0), &ReadingLength::Chapters(6)),
            Some(0.5)
        );
        // A percentage needs no length at all, and is clamped.
        assert_eq!(
            progression_to_normalized(&Progression::Percentage(0.25), &ReadingLength::Cfi),
            Some(0.25)
        );
        assert_eq!(
            progression_to_normalized(&Progression::Percentage(9.0), &ReadingLength::Cfi),
            Some(1.0)
        );
        // A CFI position has no numeric whole to divide by.
        assert_eq!(
            progression_to_normalized(
                &Progression::Cfi("epubcfi(/6/4)".into()),
                &ReadingLength::Cfi
            ),
            None
        );
        // Mismatched pairs do not guess.
        assert_eq!(
            progression_to_normalized(&Progression::Page(5), &ReadingLength::Seconds(10)),
            None
        );
    }
}
