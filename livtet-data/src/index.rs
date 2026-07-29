use strum::{Display, EnumIter, EnumString, IntoEnumIterator};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Display, EnumIter, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum NamedIndex {
    #[strum(to_string = "idx_search_history_searched_at")]
    SearchHistorySearchedAt,
    #[strum(to_string = "idx_identifiers_source")]
    IdentifierSource,
}

impl NamedIndex {
    pub fn all() -> impl Iterator<Item = Self> {
        Self::iter()
    }
}
