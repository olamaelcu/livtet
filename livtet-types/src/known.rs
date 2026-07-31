use serde::{Deserialize, Serialize};
use specta::Type;
use ulid::Ulid;

use crate::ProgressUnit;

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
    /// Uses a fixed timestamp (May 11, 2026 00:00:00 UTC) and the enum
    /// discriminant as the random component.
    pub fn ulid(self) -> Ulid {
        let rand = self as u128;
        Ulid::from_parts(crate::KNOWN_FORMAT_TIME_MS, rand)
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
    pub fn schema(self) -> crate::FormatMetadataSchema {
        match self {
            Self::Hardcover | Self::TradePaperback | Self::MassMarketPaperback => {
                crate::FormatMetadataSchema::PhysicalBook
            }
            Self::Ebook | Self::Pdf | Self::Epub | Self::Mobi => crate::FormatMetadataSchema::Ebook,
            Self::Audiobook => crate::FormatMetadataSchema::Audiobook,
        }
    }

    /// Returns the default progress unit for this format.
    ///
    /// - Physical books (Hardcover, TradePaperback, MassMarketPaperback, Pdf) → "page"
    /// - Digital text (Ebook, Epub, Mobi) → "virtual_page"
    /// - Audio (Audiobook) → "timestamp"
    pub fn default_progress_unit(self) -> &'static str {
        match self {
            Self::Hardcover | Self::TradePaperback | Self::MassMarketPaperback | Self::Pdf => {
                "page"
            }
            Self::Ebook | Self::Epub | Self::Mobi => "virtual_page",
            Self::Audiobook => "timestamp",
        }
    }

    /// Returns the default progress unit as a ProgressUnit enum.
    pub fn default_progress_unit_enum(self) -> ProgressUnit {
        match self.default_progress_unit() {
            "page" => ProgressUnit::Page,
            "virtual_page" => ProgressUnit::VirtualPage,
            "timestamp" => ProgressUnit::Timestamp,
            _ => ProgressUnit::Percentage,
        }
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
    /// Uses a fixed timestamp (May 11, 2026 00:01:00 UTC) and the enum
    /// discriminant as the random component.
    pub fn ulid(self) -> Ulid {
        let rand = self as u128;
        Ulid::from_parts(crate::KNOWN_LANGUAGE_TIME_MS, rand)
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

    /// Normalize an arbitrary ISO 639 language code into a canonical
    /// 2-letter ISO 639-1 code.  Handles 639-1 (`"en"`), 639-3
    /// (`"eng"`), and BCP 47 locale strings (`"en-GB"`).
    ///
    /// Returns `None` when `isolang` does not recognize the input.
    pub fn normalize_language_code(raw: &str) -> Option<String> {
        let normalized = raw.replace('_', "-");
        if let Some(lang) = isolang::Language::from_locale(&normalized) {
            return lang.to_639_1().map(|s| s.to_owned());
        }
        if let Some(lang) = isolang::Language::from_639_3(&normalized) {
            return lang.to_639_1().map(|s| s.to_owned());
        }
        isolang::Language::from_639_1(&normalized)
            .and_then(|l| l.to_639_1())
            .map(|s| s.to_owned())
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

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize)]
#[repr(u16)]
pub enum KnownGenres {
    Fiction = 200,
    NonFiction = 201,
    LiteraryFiction = 202,
    HistoricalFiction = 203,
    Fantasy = 204,
    ScienceFiction = 205,
    Romance = 206,
    Mystery = 207,
    Thriller = 208,
    Horror = 209,
    Paranormal = 210,
    Dystopian = 211,
    Adventure = 212,
    Contemporary = 213,
    Classics = 214,
    ShortStories = 215,
    Poetry = 216,
    Drama = 217,
    GraphicNovels = 218,
    Humor = 219,
    LgbtqPlus = 220,
    UrbanFantasy = 221,
    MagicalRealism = 222,
    FairyTalesFolklore = 223,
    Gothic = 224,
    Satire = 225,
    WomensFiction = 226,
    Essay = 227,
    SpeculativeFiction = 228,
    YoungAdult = 229,
    Childrens = 230,
    BiographyMemoir = 231,
    History = 232,
    SelfHelp = 233,
    ScienceNature = 234,
    Travel = 235,
    TrueCrime = 236,
    NoirCrime = 237,
    Western = 238,
    CozyMystery = 239,
    AfricanLiterature = 240,
    AfricanAmericanLiterature = 241,
    AsianLiterature = 242,
    LatinAmericanLiterature = 243,
    MiddleEasternLiterature = 244,
    CaribbeanLiterature = 245,
    IndigenousLiterature = 246,
    WorldLiterature = 247,
    Theology = 248,
    ChristianLiterature = 249,
    JewishLiterature = 250,
    IslamicLiterature = 251,
    BuddhistLiterature = 252,
    HinduLiterature = 253,
    ComparativeReligion = 254,
    EpicFantasy = 255,
    ClimateFiction = 256,
    NewAdult = 257,
}

impl KnownGenres {
    pub fn all() -> Vec<Self> {
        vec![
            Self::Fiction,
            Self::NonFiction,
            Self::LiteraryFiction,
            Self::HistoricalFiction,
            Self::Fantasy,
            Self::ScienceFiction,
            Self::Romance,
            Self::Mystery,
            Self::Thriller,
            Self::Horror,
            Self::Paranormal,
            Self::Dystopian,
            Self::Adventure,
            Self::Contemporary,
            Self::Classics,
            Self::ShortStories,
            Self::Poetry,
            Self::Drama,
            Self::GraphicNovels,
            Self::Humor,
            Self::LgbtqPlus,
            Self::UrbanFantasy,
            Self::MagicalRealism,
            Self::FairyTalesFolklore,
            Self::Gothic,
            Self::Satire,
            Self::WomensFiction,
            Self::Essay,
            Self::SpeculativeFiction,
            Self::YoungAdult,
            Self::Childrens,
            Self::BiographyMemoir,
            Self::History,
            Self::SelfHelp,
            Self::ScienceNature,
            Self::Travel,
            Self::TrueCrime,
            Self::NoirCrime,
            Self::Western,
            Self::CozyMystery,
            Self::AfricanLiterature,
            Self::AfricanAmericanLiterature,
            Self::AsianLiterature,
            Self::LatinAmericanLiterature,
            Self::MiddleEasternLiterature,
            Self::CaribbeanLiterature,
            Self::IndigenousLiterature,
            Self::WorldLiterature,
            Self::Theology,
            Self::ChristianLiterature,
            Self::JewishLiterature,
            Self::IslamicLiterature,
            Self::BuddhistLiterature,
            Self::HinduLiterature,
            Self::ComparativeReligion,
            Self::EpicFantasy,
            Self::ClimateFiction,
            Self::NewAdult,
        ]
    }

    /// Returns the deterministic ULID for this genre.
    /// Uses a fixed timestamp (May 11, 2026 00:02:00 UTC) and the enum
    /// discriminant as the random component.
    pub fn ulid(self) -> Ulid {
        let rand = self as u128;
        Ulid::from_parts(crate::KNOWN_GENRE_TIME_MS, rand)
    }

    /// Returns the human-readable display name for this genre.
    pub fn name(self) -> &'static str {
        match self {
            Self::Fiction => "Fiction",
            Self::NonFiction => "Non-Fiction",
            Self::LiteraryFiction => "Literary Fiction",
            Self::HistoricalFiction => "Historical Fiction",
            Self::Fantasy => "Fantasy",
            Self::ScienceFiction => "Science Fiction",
            Self::Romance => "Romance",
            Self::Mystery => "Mystery",
            Self::Thriller => "Thriller",
            Self::Horror => "Horror",
            Self::Paranormal => "Paranormal",
            Self::Dystopian => "Dystopian",
            Self::Adventure => "Adventure",
            Self::Contemporary => "Contemporary",
            Self::Classics => "Classics",
            Self::ShortStories => "Short Stories",
            Self::Poetry => "Poetry",
            Self::Drama => "Drama",
            Self::GraphicNovels => "Graphic Novels",
            Self::Humor => "Humor",
            Self::LgbtqPlus => "LGBTQ+",
            Self::UrbanFantasy => "Urban Fantasy",
            Self::MagicalRealism => "Magical Realism",
            Self::FairyTalesFolklore => "Fairy Tales / Folklore",
            Self::Gothic => "Gothic",
            Self::Satire => "Satire",
            Self::WomensFiction => "Women's Fiction",
            Self::Essay => "Essay",
            Self::SpeculativeFiction => "Speculative Fiction",
            Self::YoungAdult => "Young Adult",
            Self::Childrens => "Children's",
            Self::BiographyMemoir => "Biography / Memoir",
            Self::History => "History",
            Self::SelfHelp => "Self-Help",
            Self::ScienceNature => "Science / Nature",
            Self::Travel => "Travel",
            Self::TrueCrime => "True Crime",
            Self::NoirCrime => "Noir / Crime",
            Self::Western => "Western",
            Self::CozyMystery => "Cozy Mystery",
            Self::AfricanLiterature => "African Literature",
            Self::AfricanAmericanLiterature => "African American Literature",
            Self::AsianLiterature => "Asian Literature",
            Self::LatinAmericanLiterature => "Latin American Literature",
            Self::MiddleEasternLiterature => "Middle Eastern Literature",
            Self::CaribbeanLiterature => "Caribbean Literature",
            Self::IndigenousLiterature => "Indigenous Literature",
            Self::WorldLiterature => "World Literature",
            Self::Theology => "Theology",
            Self::ChristianLiterature => "Christian Literature",
            Self::JewishLiterature => "Jewish Literature",
            Self::IslamicLiterature => "Islamic Literature",
            Self::BuddhistLiterature => "Buddhist Literature",
            Self::HinduLiterature => "Hindu Literature",
            Self::ComparativeReligion => "Comparative Religion",
            Self::EpicFantasy => "Epic Fantasy",
            Self::ClimateFiction => "Climate Fiction",
            Self::NewAdult => "New Adult",
        }
    }
}

impl From<KnownGenres> for Ulid {
    fn from(val: KnownGenres) -> Self {
        val.ulid()
    }
}

impl From<KnownGenres> for crate::DbId {
    fn from(val: KnownGenres) -> Self {
        crate::DbId(val.ulid())
    }
}

impl From<Ulid> for KnownGenres {
    fn from(ulid: Ulid) -> Self {
        let rand = ulid.random();
        match rand {
            200 => KnownGenres::Fiction,
            201 => KnownGenres::NonFiction,
            202 => KnownGenres::LiteraryFiction,
            203 => KnownGenres::HistoricalFiction,
            204 => KnownGenres::Fantasy,
            205 => KnownGenres::ScienceFiction,
            206 => KnownGenres::Romance,
            207 => KnownGenres::Mystery,
            208 => KnownGenres::Thriller,
            209 => KnownGenres::Horror,
            210 => KnownGenres::Paranormal,
            211 => KnownGenres::Dystopian,
            212 => KnownGenres::Adventure,
            213 => KnownGenres::Contemporary,
            214 => KnownGenres::Classics,
            215 => KnownGenres::ShortStories,
            216 => KnownGenres::Poetry,
            217 => KnownGenres::Drama,
            218 => KnownGenres::GraphicNovels,
            219 => KnownGenres::Humor,
            220 => KnownGenres::LgbtqPlus,
            221 => KnownGenres::UrbanFantasy,
            222 => KnownGenres::MagicalRealism,
            223 => KnownGenres::FairyTalesFolklore,
            224 => KnownGenres::Gothic,
            225 => KnownGenres::Satire,
            226 => KnownGenres::WomensFiction,
            227 => KnownGenres::Essay,
            228 => KnownGenres::SpeculativeFiction,
            229 => KnownGenres::YoungAdult,
            230 => KnownGenres::Childrens,
            231 => KnownGenres::BiographyMemoir,
            232 => KnownGenres::History,
            233 => KnownGenres::SelfHelp,
            234 => KnownGenres::ScienceNature,
            235 => KnownGenres::Travel,
            236 => KnownGenres::TrueCrime,
            237 => KnownGenres::NoirCrime,
            238 => KnownGenres::Western,
            239 => KnownGenres::CozyMystery,
            240 => KnownGenres::AfricanLiterature,
            241 => KnownGenres::AfricanAmericanLiterature,
            242 => KnownGenres::AsianLiterature,
            243 => KnownGenres::LatinAmericanLiterature,
            244 => KnownGenres::MiddleEasternLiterature,
            245 => KnownGenres::CaribbeanLiterature,
            246 => KnownGenres::IndigenousLiterature,
            247 => KnownGenres::WorldLiterature,
            248 => KnownGenres::Theology,
            249 => KnownGenres::ChristianLiterature,
            250 => KnownGenres::JewishLiterature,
            251 => KnownGenres::IslamicLiterature,
            252 => KnownGenres::BuddhistLiterature,
            253 => KnownGenres::HinduLiterature,
            254 => KnownGenres::ComparativeReligion,
            255 => KnownGenres::EpicFantasy,
            256 => KnownGenres::ClimateFiction,
            257 => KnownGenres::NewAdult,
            _ => panic!("Unknown ULID for KnownGenres: {ulid}"),
        }
    }
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize)]
#[repr(u16)]
pub enum KnownSubjects {
    WorldHistory = 600,
    AfricanHistory = 601,
    AfricanAmericanHistory = 602,
    AsianHistory = 603,
    EuropeanHistory = 604,
    LatinAmericanHistory = 605,
    MiddleEasternHistory = 606,
    AmericanHistory = 607,
    AncientHistory = 608,
    MilitaryHistory = 609,
    ReligiousHistory = 610,
    HistoryOfChristianity = 611,
    Sociology = 612,
    Anthropology = 613,
    Archaeology = 614,
    CulturalStudies = 615,
    AfricanStudies = 616,
    AfricanAmericanStudies = 617,
    AsianStudies = 618,
    LatinAmericanStudies = 619,
    IndigenousStudies = 620,
    GenderStudies = 621,
    LgbtqPlusStudies = 622,
    PoliticalScience = 623,
    Economics = 624,
    Education = 625,
    Linguistics = 626,
    FolkloreMythology = 627,
    Biology = 628,
    Physics = 629,
    Chemistry = 630,
    Astronomy = 631,
    EnvironmentalScience = 632,
    Mathematics = 633,
    ComputerScience = 634,
    MedicineHealth = 635,
    ArtHistory = 636,
    FilmCinema = 637,
    Music = 638,
    Photography = 639,
    Architecture = 640,
    Design = 641,
    BusinessManagement = 642,
    Law = 643,
    Psychology = 644,
    CookingFood = 645,
    GardeningNature = 646,
    SportsRecreation = 647,
    TravelGeography = 648,
    LanguageLearning = 649,
    WritingPublishing = 650,
    Christianity = 651,
    Judaism = 652,
    Islam = 653,
    Hinduism = 654,
    Buddhism = 655,
    IndigenousSpirituality = 656,
    ComparativeReligion = 657,
    Theology = 658,
}

impl KnownSubjects {
    pub fn all() -> Vec<Self> {
        vec![
            Self::WorldHistory,
            Self::AfricanHistory,
            Self::AfricanAmericanHistory,
            Self::AsianHistory,
            Self::EuropeanHistory,
            Self::LatinAmericanHistory,
            Self::MiddleEasternHistory,
            Self::AmericanHistory,
            Self::AncientHistory,
            Self::MilitaryHistory,
            Self::ReligiousHistory,
            Self::HistoryOfChristianity,
            Self::Sociology,
            Self::Anthropology,
            Self::Archaeology,
            Self::CulturalStudies,
            Self::AfricanStudies,
            Self::AfricanAmericanStudies,
            Self::AsianStudies,
            Self::LatinAmericanStudies,
            Self::IndigenousStudies,
            Self::GenderStudies,
            Self::LgbtqPlusStudies,
            Self::PoliticalScience,
            Self::Economics,
            Self::Education,
            Self::Linguistics,
            Self::FolkloreMythology,
            Self::Biology,
            Self::Physics,
            Self::Chemistry,
            Self::Astronomy,
            Self::EnvironmentalScience,
            Self::Mathematics,
            Self::ComputerScience,
            Self::MedicineHealth,
            Self::ArtHistory,
            Self::FilmCinema,
            Self::Music,
            Self::Photography,
            Self::Architecture,
            Self::Design,
            Self::BusinessManagement,
            Self::Law,
            Self::Psychology,
            Self::CookingFood,
            Self::GardeningNature,
            Self::SportsRecreation,
            Self::TravelGeography,
            Self::LanguageLearning,
            Self::WritingPublishing,
            Self::Christianity,
            Self::Judaism,
            Self::Islam,
            Self::Hinduism,
            Self::Buddhism,
            Self::IndigenousSpirituality,
            Self::ComparativeReligion,
            Self::Theology,
        ]
    }

    /// Returns the deterministic ULID for this subject.
    /// Uses a fixed timestamp (May 11, 2026 00:05:00 UTC) and the enum
    /// discriminant as the random component.
    pub fn ulid(self) -> Ulid {
        let rand = self as u128;
        Ulid::from_parts(crate::KNOWN_SUBJECT_TIME_MS, rand)
    }

    /// Returns the human-readable display name for this subject.
    pub fn name(self) -> &'static str {
        match self {
            Self::WorldHistory => "World History",
            Self::AfricanHistory => "African History",
            Self::AfricanAmericanHistory => "African American History",
            Self::AsianHistory => "Asian History",
            Self::EuropeanHistory => "European History",
            Self::LatinAmericanHistory => "Latin American History",
            Self::MiddleEasternHistory => "Middle Eastern History",
            Self::AmericanHistory => "American History",
            Self::AncientHistory => "Ancient History",
            Self::MilitaryHistory => "Military History",
            Self::ReligiousHistory => "Religious History",
            Self::HistoryOfChristianity => "History of Christianity",
            Self::Sociology => "Sociology",
            Self::Anthropology => "Anthropology",
            Self::Archaeology => "Archaeology",
            Self::CulturalStudies => "Cultural Studies",
            Self::AfricanStudies => "African Studies",
            Self::AfricanAmericanStudies => "African American Studies",
            Self::AsianStudies => "Asian Studies",
            Self::LatinAmericanStudies => "Latin American Studies",
            Self::IndigenousStudies => "Indigenous Studies",
            Self::GenderStudies => "Gender Studies",
            Self::LgbtqPlusStudies => "LGBTQ+ Studies",
            Self::PoliticalScience => "Political Science",
            Self::Economics => "Economics",
            Self::Education => "Education",
            Self::Linguistics => "Linguistics",
            Self::FolkloreMythology => "Folklore / Mythology",
            Self::Biology => "Biology",
            Self::Physics => "Physics",
            Self::Chemistry => "Chemistry",
            Self::Astronomy => "Astronomy",
            Self::EnvironmentalScience => "Environmental Science",
            Self::Mathematics => "Mathematics",
            Self::ComputerScience => "Computer Science",
            Self::MedicineHealth => "Medicine / Health",
            Self::ArtHistory => "Art History",
            Self::FilmCinema => "Film / Cinema",
            Self::Music => "Music",
            Self::Photography => "Photography",
            Self::Architecture => "Architecture",
            Self::Design => "Design",
            Self::BusinessManagement => "Business / Management",
            Self::Law => "Law",
            Self::Psychology => "Psychology",
            Self::CookingFood => "Cooking / Food",
            Self::GardeningNature => "Gardening / Nature",
            Self::SportsRecreation => "Sports / Recreation",
            Self::TravelGeography => "Travel / Geography",
            Self::LanguageLearning => "Language Learning",
            Self::WritingPublishing => "Writing / Publishing",
            Self::Christianity => "Christianity",
            Self::Judaism => "Judaism",
            Self::Islam => "Islam",
            Self::Hinduism => "Hinduism",
            Self::Buddhism => "Buddhism",
            Self::IndigenousSpirituality => "Indigenous Spirituality",
            Self::ComparativeReligion => "Comparative Religion",
            Self::Theology => "Theology",
        }
    }
}

impl From<KnownSubjects> for Ulid {
    fn from(val: KnownSubjects) -> Self {
        val.ulid()
    }
}

impl From<KnownSubjects> for crate::DbId {
    fn from(val: KnownSubjects) -> Self {
        crate::DbId(val.ulid())
    }
}

impl From<Ulid> for KnownSubjects {
    fn from(ulid: Ulid) -> Self {
        let rand = ulid.random();
        match rand {
            600 => KnownSubjects::WorldHistory,
            601 => KnownSubjects::AfricanHistory,
            602 => KnownSubjects::AfricanAmericanHistory,
            603 => KnownSubjects::AsianHistory,
            604 => KnownSubjects::EuropeanHistory,
            605 => KnownSubjects::LatinAmericanHistory,
            606 => KnownSubjects::MiddleEasternHistory,
            607 => KnownSubjects::AmericanHistory,
            608 => KnownSubjects::AncientHistory,
            609 => KnownSubjects::MilitaryHistory,
            610 => KnownSubjects::ReligiousHistory,
            611 => KnownSubjects::HistoryOfChristianity,
            612 => KnownSubjects::Sociology,
            613 => KnownSubjects::Anthropology,
            614 => KnownSubjects::Archaeology,
            615 => KnownSubjects::CulturalStudies,
            616 => KnownSubjects::AfricanStudies,
            617 => KnownSubjects::AfricanAmericanStudies,
            618 => KnownSubjects::AsianStudies,
            619 => KnownSubjects::LatinAmericanStudies,
            620 => KnownSubjects::IndigenousStudies,
            621 => KnownSubjects::GenderStudies,
            622 => KnownSubjects::LgbtqPlusStudies,
            623 => KnownSubjects::PoliticalScience,
            624 => KnownSubjects::Economics,
            625 => KnownSubjects::Education,
            626 => KnownSubjects::Linguistics,
            627 => KnownSubjects::FolkloreMythology,
            628 => KnownSubjects::Biology,
            629 => KnownSubjects::Physics,
            630 => KnownSubjects::Chemistry,
            631 => KnownSubjects::Astronomy,
            632 => KnownSubjects::EnvironmentalScience,
            633 => KnownSubjects::Mathematics,
            634 => KnownSubjects::ComputerScience,
            635 => KnownSubjects::MedicineHealth,
            636 => KnownSubjects::ArtHistory,
            637 => KnownSubjects::FilmCinema,
            638 => KnownSubjects::Music,
            639 => KnownSubjects::Photography,
            640 => KnownSubjects::Architecture,
            641 => KnownSubjects::Design,
            642 => KnownSubjects::BusinessManagement,
            643 => KnownSubjects::Law,
            644 => KnownSubjects::Psychology,
            645 => KnownSubjects::CookingFood,
            646 => KnownSubjects::GardeningNature,
            647 => KnownSubjects::SportsRecreation,
            648 => KnownSubjects::TravelGeography,
            649 => KnownSubjects::LanguageLearning,
            650 => KnownSubjects::WritingPublishing,
            651 => KnownSubjects::Christianity,
            652 => KnownSubjects::Judaism,
            653 => KnownSubjects::Islam,
            654 => KnownSubjects::Hinduism,
            655 => KnownSubjects::Buddhism,
            656 => KnownSubjects::IndigenousSpirituality,
            657 => KnownSubjects::ComparativeReligion,
            658 => KnownSubjects::Theology,
            _ => panic!("Unknown ULID for KnownSubjects: {ulid}"),
        }
    }
}

/// Known reading source identifiers used across the codebase.
/// These are deterministic ULIDs that are seeded into the database.
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize)]
#[repr(u16)]
pub enum KnownReadingSources {
    Manual = 1,
    Koreader = 2,
    Kobo = 3,
    Kindle = 4,
    LivtetMobileIos = 5,
    LivtetMobileAndroid = 6,
    LivtetDesktop = 7,
}

impl KnownReadingSources {
    pub fn all() -> Vec<Self> {
        vec![
            Self::Manual,
            Self::Koreader,
            Self::Kobo,
            Self::Kindle,
            Self::LivtetMobileIos,
            Self::LivtetMobileAndroid,
            Self::LivtetDesktop,
        ]
    }

    /// Returns the deterministic ULID for this source.
    /// Uses a fixed timestamp (May 11, 2026 00:06:00 UTC) and the enum
    /// discriminant as the random component.
    pub fn ulid(self) -> Ulid {
        let rand = self as u128;
        Ulid::from_parts(crate::KNOWN_READING_SOURCE_TIME_MS, rand)
    }

    /// Returns the human-readable display name for this source.
    pub fn name(self) -> &'static str {
        match self {
            KnownReadingSources::Manual => "Manual entry",
            KnownReadingSources::Koreader => "KOReader",
            KnownReadingSources::Kobo => "Kobo",
            KnownReadingSources::Kindle => "Kindle",
            KnownReadingSources::LivtetMobileIos => "Livtet iOS",
            KnownReadingSources::LivtetMobileAndroid => "Livtet Android",
            KnownReadingSources::LivtetDesktop => "Livtet Desktop",
        }
    }

    /// Returns the URN prefix for this source.
    pub fn urn(self) -> String {
        match self {
            KnownReadingSources::Manual => "urn:manual".to_string(),
            KnownReadingSources::Koreader => "urn:koreader:default".to_string(),
            KnownReadingSources::Kobo => "urn:kobo:default".to_string(),
            KnownReadingSources::Kindle => "urn:kindle:default".to_string(),
            KnownReadingSources::LivtetMobileIos => "urn:livtet:mobile:ios:default".to_string(),
            KnownReadingSources::LivtetMobileAndroid => {
                "urn:livtet:mobile:android:default".to_string()
            }
            KnownReadingSources::LivtetDesktop => "urn:livtet:desktop:default".to_string(),
        }
    }

    /// Returns the emoji for this source.
    pub fn emoji(self) -> &'static str {
        match self {
            KnownReadingSources::Manual => "\u{270F}",           // ✏️
            KnownReadingSources::Koreader => "\u{1F4D6}",        // 📖
            KnownReadingSources::Kobo => "\u{1F4DA}",            // 📚
            KnownReadingSources::Kindle => "\u{1F525}",          // 🔥
            KnownReadingSources::LivtetMobileIos => "\u{1F4F1}", // 📱
            KnownReadingSources::LivtetMobileAndroid => "\u{1F916}", // 🤖
            KnownReadingSources::LivtetDesktop => "\u{1F5A5}",   // 🖥
        }
    }

    /// Returns the color for this source.
    pub fn color(self) -> &'static str {
        match self {
            KnownReadingSources::Manual => "#6B7280",
            KnownReadingSources::Koreader => "#10B981",
            KnownReadingSources::Kobo => "#3B82F6",
            KnownReadingSources::Kindle => "#F59E0B",
            KnownReadingSources::LivtetMobileIos => "#111827",
            KnownReadingSources::LivtetMobileAndroid => "#22C55E",
            KnownReadingSources::LivtetDesktop => "#6366F1",
        }
    }
}

impl From<KnownReadingSources> for Ulid {
    fn from(val: KnownReadingSources) -> Self {
        val.ulid()
    }
}

impl From<KnownReadingSources> for crate::DbId {
    fn from(val: KnownReadingSources) -> Self {
        crate::DbId(val.ulid())
    }
}

impl From<Ulid> for KnownReadingSources {
    fn from(ulid: Ulid) -> Self {
        let rand = ulid.random();
        match rand {
            1 => KnownReadingSources::Manual,
            2 => KnownReadingSources::Koreader,
            3 => KnownReadingSources::Kobo,
            4 => KnownReadingSources::Kindle,
            5 => KnownReadingSources::LivtetMobileIos,
            6 => KnownReadingSources::LivtetMobileAndroid,
            7 => KnownReadingSources::LivtetDesktop,
            _ => panic!("Unknown ULID for KnownReadingSources: {ulid}"),
        }
    }
}
