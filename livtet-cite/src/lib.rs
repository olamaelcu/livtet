//! CSL-JSON-first citation data for Livtet editions.
//!
//! [`CslItem`] is the canonical representation: a Citation Style Language
//! JSON item with `type: "book"`. The renderers [`to_bibtex`],
//! [`to_bibtex_batch`] and [`to_ris`] generate their output from the CSL
//! struct, so BibTeX, RIS and CSL-JSON can never drift apart.
//!
//! The crate performs no database access. Callers load catalog rows
//! themselves and map them onto [`CitationRows`], which
//! [`CslItem::from_rows`] turns into the canonical item.
//!
//! ```
//! use livtet_cite::{
//!     CitationContributor, CitationEdition, CitationRows, ContributorRole, CslItem, to_bibtex,
//! };
//!
//! let contributors = [CitationContributor {
//!     role: ContributorRole::Author,
//!     name: Some("Ursula K. Le Guin"),
//!     family: Some("Le Guin"),
//!     given: Some("Ursula K."),
//! }];
//! let rows = CitationRows {
//!     authors: &contributors,
//!     edition: CitationEdition {
//!         title: Some("The Left Hand of Darkness"),
//!         published_year: Some(1969),
//!         ..CitationEdition::default()
//!     },
//!     ..CitationRows::default()
//! };
//!
//! let item = CslItem::from_rows("urn:livtet:edition/01H", &rows);
//! assert_eq!(item.item_type, "book");
//! assert!(to_bibtex(&item).contains("@book{leguin1969,"));
//! ```

pub mod bibtex;
pub mod csl;
pub mod key;
pub mod ris;

pub use bibtex::{to_bibtex, to_bibtex_batch};
pub use csl::{
    CitationContributor, CitationEdition, CitationIdentifier, CitationLanguage, CitationPublisher,
    CitationRows, ContributorRole, CslDate, CslItem, CslName,
};
pub use key::citation_key;
pub use ris::to_ris;
