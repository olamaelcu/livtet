use serde::{Deserialize, Serialize};
use specta::Type;
use ulid::Ulid;

use crate::FormatMetadataSchema;

/// Known format identifiers used across the codebase.
/// These are deterministic ULIDs that are seeded into the database.
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize)]
#[repr(u16)]
pub enum KnownFormats {
    Hardcover = 1,
    TradePaperback = 2,
    MassMarketPaperback = 3,
    Ebook = 4,
    Audiobook = 5,
    Pdf = 6,
    Epub = 7,
    Mobi = 8,
}

const FORMAT_TIME_MS: u64 = 1735689600000u64;
const LANGUAGE_TIME_MS: u64 = 1735689600001u64;

impl KnownFormats {
    pub fn all() -> Vec<Self> {
        vec![
            Self::Hardcover,
            Self::TradePaperback,
            Self::MassMarketPaperback,
            Self::Ebook,
            Self::Audiobook,
            Self::Pdf,
            Self::Epub,
            Self::Mobi,
        ]
    }
    /// Returns the deterministic ULID for this format.
    /// Uses a fixed timestamp (Jan 1, 2025 00:00:00 UTC) and the enum
    /// discriminant as the random component.
    pub fn ulid(self) -> Ulid {
        let rand = self as u128;
        Ulid::from_parts(FORMAT_TIME_MS, rand)
    }

    /// Returns the human-readable display name for this format.
    pub fn name(self) -> &'static str {
        match self {
            KnownFormats::Hardcover => "Hardcover",
            KnownFormats::TradePaperback => "Trade Paperback",
            KnownFormats::MassMarketPaperback => "Mass Market Paperback",
            KnownFormats::Ebook => "eBook",
            KnownFormats::Audiobook => "Audiobook",
            KnownFormats::Pdf => "PDF",
            KnownFormats::Epub => "EPUB",
            KnownFormats::Mobi => "MOBI",
        }
    }
    pub fn schema(self) -> serde_json::Value {
        let schema: FormatMetadataSchema = match self {
            Self::Hardcover | Self::TradePaperback | Self::MassMarketPaperback => {
                FormatMetadataSchema::PhysicalBook
            }
            Self::Ebook | Self::Pdf | Self::Epub | Self::Mobi => FormatMetadataSchema::Ebook,
            Self::Audiobook => FormatMetadataSchema::Audiobook,
        };
        schema.into()
    }
}

impl From<KnownFormats> for Ulid {
    fn from(val: KnownFormats) -> Self {
        val.ulid()
    }
}

impl From<KnownFormats> for crate::DbId {
    fn from(val: KnownFormats) -> Self {
        crate::DbId(val.ulid())
    }
}

impl From<Ulid> for KnownFormats {
    fn from(ulid: Ulid) -> Self {
        let rand = ulid.random();
        match rand {
            1 => KnownFormats::Hardcover,
            2 => KnownFormats::TradePaperback,
            3 => KnownFormats::MassMarketPaperback,
            4 => KnownFormats::Ebook,
            5 => KnownFormats::Audiobook,
            6 => KnownFormats::Pdf,
            7 => KnownFormats::Epub,
            8 => KnownFormats::Mobi,
            _ => panic!("Unknown ULID for KnownFormats: {ulid}"),
        }
    }
}

/// Known language identifiers used across the codebase.
/// These are deterministic ULIDs that are seeded into the database.
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize)]
#[repr(u16)]
pub enum CommonLanguages {
    English = 100,
    Spanish = 101,
    HaitianKreyol = 102,
    UkEnglish = 103,
    Russian = 104,
    Japanese = 105,
    MandarinChinese = 106,
    Hindi = 107,
    Arabic = 108,
    Bengali = 109,
    Portuguese = 110,
    WesternPunjabi = 111,
    Turkish = 112,
    Vietnamese = 113,
    French = 114,
    German = 115,
    Korean = 116,
    Italian = 117,
    Indonesian = 118,
    Dutch = 119,
    Polish = 120,
    Swedish = 121,
    Ukrainian = 122,
    Persian = 123,
    Urdu = 124,
    Hebrew = 125,
    Thai = 126,
    Tagalog = 127,
    Tamil = 128,
    Telugu = 129,
}

impl CommonLanguages {
    pub fn all() -> Vec<Self> {
        vec![
            Self::English,
            Self::Spanish,
            Self::HaitianKreyol,
            Self::UkEnglish,
            Self::Russian,
            Self::Japanese,
            Self::MandarinChinese,
            Self::Hindi,
            Self::Arabic,
            Self::Bengali,
            Self::Portuguese,
            Self::WesternPunjabi,
            Self::Turkish,
            Self::Vietnamese,
            Self::French,
            Self::German,
            Self::Korean,
            Self::Italian,
            Self::Indonesian,
            Self::Dutch,
            Self::Polish,
            Self::Swedish,
            Self::Ukrainian,
            Self::Persian,
            Self::Urdu,
            Self::Hebrew,
            Self::Thai,
            Self::Tagalog,
            Self::Tamil,
            Self::Telugu,
        ]
    }

    /// Returns the deterministic ULID for this language.
    /// Uses a fixed timestamp (Jan 1, 2025 00:00:00 UTC) and the enum
    /// discriminant as the random component.
    pub fn ulid(self) -> Ulid {
        let rand = self as u128;
        Ulid::from_parts(LANGUAGE_TIME_MS, rand)
    }

    /// Returns the human-readable display name for this language.
    pub fn name(self) -> &'static str {
        match self {
            CommonLanguages::English => "English",
            CommonLanguages::Spanish => "Spanish",
            CommonLanguages::HaitianKreyol => "Haitian Kreyol",
            CommonLanguages::UkEnglish => "UK English",
            CommonLanguages::Russian => "Russian",
            CommonLanguages::Japanese => "Japanese",
            CommonLanguages::MandarinChinese => "Chinese (Mandarin)",
            CommonLanguages::Hindi => "Hindi",
            CommonLanguages::Arabic => "Arabic",
            CommonLanguages::Bengali => "Bengali",
            CommonLanguages::Portuguese => "Portuguese",
            CommonLanguages::WesternPunjabi => "Punjabi (Western)",
            CommonLanguages::Turkish => "Turkish",
            CommonLanguages::Vietnamese => "Vietnamese",
            CommonLanguages::French => "French",
            CommonLanguages::German => "German",
            CommonLanguages::Korean => "Korean",
            CommonLanguages::Italian => "Italian",
            CommonLanguages::Indonesian => "Indonesian",
            CommonLanguages::Dutch => "Dutch",
            CommonLanguages::Polish => "Polish",
            CommonLanguages::Swedish => "Swedish",
            CommonLanguages::Ukrainian => "Ukrainian",
            CommonLanguages::Persian => "Persian (Farsi)",
            CommonLanguages::Urdu => "Urdu",
            CommonLanguages::Hebrew => "Hebrew",
            CommonLanguages::Thai => "Thai",
            CommonLanguages::Tagalog => "Tagalog (Filipino)",
            CommonLanguages::Tamil => "Tamil",
            CommonLanguages::Telugu => "Telugu",
        }
    }

    /// Returns the ISO 639-1 or BCP 47 language code.
    pub fn code(self) -> &'static str {
        match self {
            CommonLanguages::English => "en",
            CommonLanguages::Spanish => "es",
            CommonLanguages::HaitianKreyol => "ht",
            CommonLanguages::UkEnglish => "en-GB",
            CommonLanguages::Russian => "ru",
            CommonLanguages::Japanese => "ja",
            CommonLanguages::MandarinChinese => "zh",
            CommonLanguages::Hindi => "hi",
            CommonLanguages::Arabic => "ar",
            CommonLanguages::Bengali => "bn",
            CommonLanguages::Portuguese => "pt",
            CommonLanguages::WesternPunjabi => "pnb",
            CommonLanguages::Turkish => "tr",
            CommonLanguages::Vietnamese => "vi",
            CommonLanguages::French => "fr",
            CommonLanguages::German => "de",
            CommonLanguages::Korean => "ko",
            CommonLanguages::Italian => "it",
            CommonLanguages::Indonesian => "id",
            CommonLanguages::Dutch => "nl",
            CommonLanguages::Polish => "pl",
            CommonLanguages::Swedish => "sv",
            CommonLanguages::Ukrainian => "uk",
            CommonLanguages::Persian => "fa",
            CommonLanguages::Urdu => "ur",
            CommonLanguages::Hebrew => "he",
            CommonLanguages::Thai => "th",
            CommonLanguages::Tagalog => "tl",
            CommonLanguages::Tamil => "ta",
            CommonLanguages::Telugu => "te",
        }
    }

    /// Returns the flag emoji for this language.
    pub fn flag_emoji(self) -> &'static str {
        match self {
            CommonLanguages::English => "🇺🇸",
            CommonLanguages::Spanish => "🇪🇸",
            CommonLanguages::HaitianKreyol => "🇭🇹",
            CommonLanguages::UkEnglish => "🇬🇧",
            CommonLanguages::Russian => "🇷🇺",
            CommonLanguages::Japanese => "🇯🇵",
            CommonLanguages::MandarinChinese => "🇨🇳",
            CommonLanguages::Hindi => "🇮🇳",
            CommonLanguages::Arabic => "🇵🇸",
            CommonLanguages::Bengali => "🇧🇩",
            CommonLanguages::Portuguese => "🇵🇹",
            CommonLanguages::WesternPunjabi => "🇵🇰",
            CommonLanguages::Turkish => "🇹🇷",
            CommonLanguages::Vietnamese => "🇻🇳",
            CommonLanguages::French => "🇫🇷",
            CommonLanguages::German => "🇩🇪",
            CommonLanguages::Korean => "🇰🇷",
            CommonLanguages::Italian => "🇮🇹",
            CommonLanguages::Indonesian => "🇮🇩",
            CommonLanguages::Dutch => "🇳🇱",
            CommonLanguages::Polish => "🇵🇱",
            CommonLanguages::Swedish => "🇸🇪",
            CommonLanguages::Ukrainian => "🇺🇦",
            CommonLanguages::Persian => "🇮🇷",
            CommonLanguages::Urdu => "🇵🇰",
            CommonLanguages::Hebrew => "🇮🇱",
            CommonLanguages::Thai => "🇹🇭",
            CommonLanguages::Tagalog => "🇵🇭",
            CommonLanguages::Tamil => "🇮🇳",
            CommonLanguages::Telugu => "🇮🇳",
        }
    }
}

impl From<CommonLanguages> for Ulid {
    fn from(val: CommonLanguages) -> Self {
        val.ulid()
    }
}

impl From<CommonLanguages> for crate::DbId {
    fn from(val: CommonLanguages) -> Self {
        crate::DbId(val.ulid())
    }
}

impl From<Ulid> for CommonLanguages {
    fn from(ulid: Ulid) -> Self {
        let rand = ulid.random();
        match rand {
            100 => CommonLanguages::English,
            101 => CommonLanguages::Spanish,
            102 => CommonLanguages::HaitianKreyol,
            103 => CommonLanguages::UkEnglish,
            104 => CommonLanguages::Russian,
            105 => CommonLanguages::Japanese,
            106 => CommonLanguages::MandarinChinese,
            107 => CommonLanguages::Hindi,
            108 => CommonLanguages::Arabic,
            109 => CommonLanguages::Bengali,
            110 => CommonLanguages::Portuguese,
            111 => CommonLanguages::WesternPunjabi,
            112 => CommonLanguages::Turkish,
            113 => CommonLanguages::Vietnamese,
            114 => CommonLanguages::French,
            115 => CommonLanguages::German,
            116 => CommonLanguages::Korean,
            117 => CommonLanguages::Italian,
            118 => CommonLanguages::Indonesian,
            119 => CommonLanguages::Dutch,
            120 => CommonLanguages::Polish,
            121 => CommonLanguages::Swedish,
            122 => CommonLanguages::Ukrainian,
            123 => CommonLanguages::Persian,
            124 => CommonLanguages::Urdu,
            125 => CommonLanguages::Hebrew,
            126 => CommonLanguages::Thai,
            127 => CommonLanguages::Tagalog,
            128 => CommonLanguages::Tamil,
            129 => CommonLanguages::Telugu,
            _ => panic!("Unknown ULID for KnownLanguageIds: {ulid}"),
        }
    }
}
