use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, PartialEq, Eq, Type, Serialize, Deserialize)]
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
