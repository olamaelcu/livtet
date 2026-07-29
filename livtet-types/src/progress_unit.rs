use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressUnit {
    Percentage,
    Page,
    VirtualPage,
    Timestamp,
    Chapter,
    Cfi,
}

impl ProgressUnit {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Percentage => "percentage",
            Self::Page => "page",
            Self::VirtualPage => "virtual_page",
            Self::Timestamp => "timestamp",
            Self::Chapter => "chapter",
            Self::Cfi => "cfi",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Type, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Progression {
    Percentage(f64),
    Page(u32),
    VirtualPage(u32),
    TimestampSeconds(i64),
    Chapter(f64),
    Cfi(String),
}
