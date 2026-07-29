use serde::{Deserialize, Serialize};
use specta::Type;

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize)]
#[repr(u16)]
pub enum BookCondition {
    New = 700,
    Used = 701,
    Good = 702,
    Fair = 703,
}

urn_enum!(
    BookCondition,
    crate::BOOK_CONDITION_TIME_MS,
    "urn:livtet:book-cond/";
    (New = 700, "new", "New"),
    (Used = 701, "used", "Used"),
    (Good = 702, "good", "Good"),
    (Fair = 703, "fair", "Fair"),
    all: [New, Used, Good, Fair]
);
