use serde::{Deserialize, Serialize};
use specta::Type;

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize)]
#[repr(u16)]
pub enum ContributorRole {
    Author = 600,
    Translator = 601,
    Editor = 602,
    Illustrator = 603,
    Narrator = 604,
    IntroAuthor = 605,
    ForewordAuthor = 606,
    EpilogueAuthor = 607,
}

const CONTRIBUTOR_ROLE_TIME_MS: u64 = 1735689600600u64;

urn_enum!(
    ContributorRole,
    CONTRIBUTOR_ROLE_TIME_MS,
    "urn:livtet:contrib/";
    (Author = 600, "author", "Author"),
    (Translator = 601, "translator", "Translator"),
    (Editor = 602, "editor", "Editor"),
    (Illustrator = 603, "illustrator", "Illustrator"),
    (Narrator = 604, "narrator", "Narrator"),
    (IntroAuthor = 605, "intro_author", "Intro Author"),
    (ForewordAuthor = 606, "foreword_author", "Foreword Author"),
    (EpilogueAuthor = 607, "epilogue_author", "Epilogue Author"),
    all: [
        Author,
        Translator,
        Editor,
        Illustrator,
        Narrator,
        IntroAuthor,
        ForewordAuthor,
        EpilogueAuthor
    ]
);
