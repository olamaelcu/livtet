//! Livtet foundational types — DbId, DiskPath, Urn, and migrations schema helpers
//!
//! These types are used across all Livtet crates and have minimal dependencies.
//!
//! ## Features
//!
//! - `search`: enables the `search` module with search/filter types
//! - `fake`: enables `fake::Dummy` derives for testing

/// Generate impls for a URN-backed enum.
///
/// The enum itself is defined manually at the call site (with its
/// `#[repr(u16)]` discriminant list and existing `derive`s preserved).
/// This macro only generates impl blocks.
///
/// # Generated methods
///
/// - `pub fn ulid(self) -> Ulid`
/// - `pub fn as_str(self) -> &'static str`
/// - `pub fn name(self) -> &'static str`
/// - `pub fn all() -> Vec<Self>`
/// - `impl From<Ulid> for Type`
/// - `impl From<Type> for DbId`
/// - `impl From<Type> for String`
/// - `impl From<Type> for sea_orm::Value`
///
/// # Generated sea_orm impls (only when `sea_orm { }` is present)
///
/// - `impl TryFrom<sea_orm::Value> for Type`
/// - `impl sea_orm::sea_query::ValueType for Type`
/// - `impl sea_orm::TryGetable for Type`
/// - `impl sea_orm::sea_query::Nullable for Type`
/// - `impl sea_orm::TryFromU64 for Type`
///
/// # Syntax
///
/// ```text
/// urn_enum!(
///     TypeName,
///     TIME_MS_EXPR,
///     "urn:livtet:prefix/";
///     (VariantA = 100, "variant_a", "Variant A"),
///     (VariantB = 101, "variant_b", "Variant B"),
///     all: [VariantA, VariantB],
///     sea_orm { }  // optional
/// );
/// ```
#[macro_export]
macro_rules! urn_enum {
    // Entry point WITH `sea_orm { }` block — emits the full sea_orm suite.
    (
        $ty:ident,
        $time_ms:expr,
        $urn_prefix:literal;
        $( ($variant:ident = $disc:literal, $snake:literal, $display:literal), )+ $(,)?
        all: [ $($all_variant:ident),+ $(,)? ]
        , sea_orm { $($sea_orm:tt)* }
    ) => {
        urn_enum!(@common $ty, $time_ms, $urn_prefix;
            $( ($variant = $disc, $snake, $display), )+
            all: [ $($all_variant),+ ]
        );

        urn_enum!(@sea_orm $ty, $urn_prefix; $( ($variant = $disc, $snake, $display), )+ ; $($sea_orm)*);
    };

    // Entry point WITHOUT `sea_orm { }` block — common impls only.
    (
        $ty:ident,
        $time_ms:expr,
        $urn_prefix:literal;
        $( ($variant:ident = $disc:literal, $snake:literal, $display:literal), )+ $(,)?
        all: [ $($all_variant:ident),+ $(,)? ]
    ) => {
        urn_enum!(@common $ty, $time_ms, $urn_prefix;
            $( ($variant = $disc, $snake, $display), )+
            all: [ $($all_variant),+ ]
        );
    };

    // Common impls shared between both entry points.
    (@common $ty:ident, $time_ms:expr, $urn_prefix:literal;
        $( ($variant:ident = $disc:literal, $snake:literal, $display:literal), )+
        all: [ $($all_variant:ident),+ ]
    ) => {
        impl $ty {
            /// The deterministic ULID for this variant. Uses the
            /// shared timestamp constant and the discriminant as the
            /// random component.
            pub fn ulid(self) -> ::ulid::Ulid {
                let rand = self as u128;
                ::ulid::Ulid::from_parts($time_ms, rand)
            }

            /// The canonical snake_case string form.
            pub fn as_str(self) -> &'static str {
                match self {
                    $( $ty::$variant => $snake, )+
                }
            }

            /// The human-readable label.
            pub fn name(self) -> &'static str {
                match self {
                    $( $ty::$variant => $display, )+
                }
            }

            /// All variants in discriminant order.
            pub fn all() -> Vec<Self> {
                vec![ $( Self::$all_variant ),+ ]
            }

            /// The full URN string (e.g. `urn:livtet:work/status/300`).
            pub fn to_urn(self) -> String {
                format!("{}{}", $urn_prefix, self as u16)
            }
        }

        impl From<::ulid::Ulid> for $ty {
            fn from(ulid: ::ulid::Ulid) -> Self {
                let rand = ulid.random();
                match rand {
                    $( $disc => Self::$variant, )+
                    _ => panic!("Unknown ULID for {}: {}", stringify!($ty), ulid),
                }
            }
        }

        impl From<$ty> for $crate::DbId {
            fn from(val: $ty) -> Self {
                $crate::DbId(val.ulid())
            }
        }

        impl From<$ty> for String {
            fn from(val: $ty) -> Self {
                val.to_urn()
            }
        }

        impl From<$ty> for ::sea_orm::Value {
            fn from(val: $ty) -> Self {
                ::sea_orm::Value::String(Some(val.to_urn()))
            }
        }
    };

    // Sea_orm-specific impls (only reached when entry point detected the block).
    (@sea_orm $ty:ident, $urn_prefix:literal; $( ($variant:ident = $disc:literal, $snake:literal, $display:literal), )+ ; $($body:tt)*) => {
        impl ::std::convert::TryFrom<::sea_orm::Value> for $ty {
            type Error = ::std::string::String;
            fn try_from(v: ::sea_orm::Value) -> ::std::result::Result<Self, Self::Error> {
                match v {
                    ::sea_orm::Value::String(Some(s)) => {
                        let raw = s.as_str();
                        let prefix = $urn_prefix;
                        let rest = raw.strip_prefix(prefix)
                            .ok_or_else(|| format!("expected URN with prefix {}, got {:?}", prefix, raw))?;
                        let n: u16 = rest.parse()
                            .map_err(|e| format!("failed to parse discriminant from {:?}: {}", raw, e))?;
                        match n {
                            $( $disc => ::std::result::Result::Ok(Self::$variant), )+
                            other => ::std::result::Result::Err(format!(
                                "unknown {} discriminant: {}",
                                stringify!($ty),
                                other,
                            )),
                        }
                    }
                    other => ::std::result::Result::Err(format!(
                        "expected Value::String, got {:?}",
                        other,
                    )),
                }
            }
        }

        impl ::sea_orm::sea_query::ValueType for $ty {
            fn try_from(v: ::sea_orm::Value) -> ::std::result::Result<Self, ::sea_orm::sea_query::ValueTypeErr> {
                <Self as ::std::convert::TryFrom<::sea_orm::Value>>::try_from(v)
                    .map_err(|_| ::sea_orm::sea_query::ValueTypeErr)
            }

            fn type_name() -> ::std::string::String {
                stringify!($ty).to_owned()
            }

            fn array_type() -> ::sea_orm::sea_query::ArrayType {
                ::sea_orm::sea_query::ArrayType::String
            }

            fn column_type() -> ::sea_orm::sea_query::ColumnType {
                ::sea_orm::sea_query::ColumnType::Text
            }
        }

        impl ::sea_orm::TryGetable for $ty {
            fn try_get_by<I: ::sea_orm::ColIdx>(
                res: &::sea_orm::QueryResult,
                index: I,
            ) -> ::std::result::Result<Self, ::sea_orm::TryGetError> {
                <::std::string::String as ::sea_orm::TryGetable>::try_get_by(res, index).and_then(|s| {
                    <Self as ::std::convert::TryFrom<::sea_orm::Value>>::try_from(::sea_orm::Value::String(Some(s)))
                        .map_err(|e| ::sea_orm::TryGetError::DbErr(::sea_orm::DbErr::Custom(e)))
                })
            }
        }

        impl ::sea_orm::sea_query::Nullable for $ty {
            fn null() -> ::sea_orm::Value {
                ::sea_orm::Value::String(None)
            }
        }

        impl ::sea_orm::TryFromU64 for $ty {
            fn try_from_u64(_: u64) -> ::std::result::Result<Self, ::sea_orm::DbErr> {
                ::std::result::Result::Err(::sea_orm::DbErr::Custom(
                    format!("{} cannot be created from u64", stringify!($ty)),
                ))
            }
        }
    };
}

#[cfg(feature = "fake")]
pub mod fake;

pub mod address;
pub mod book_condition;
pub mod contributor_role;
pub mod db_id;
pub mod device_type;
pub mod disk_path;
pub mod format_metadata_schema;
pub mod identifier;
pub mod identifier_kind;
pub mod isbn;
pub mod known;
pub mod pairing_status;
pub mod progress_unit;
pub mod published_date;
pub mod reading_length;
pub mod series_type;
pub mod urn;
pub mod work_filters;
pub mod work_status;

// ── Migrations schema helpers (always included) ────────────────────

pub mod iden;
pub mod migration;
pub use migration::{db_id as mig_db_id, timestamps as mig_timestamps};

// ── Feature-gated modules ─────────────────────────────────────────

#[cfg(feature = "search")]
pub mod search;
pub use address::Address;
pub use book_condition::BookCondition;
pub use contributor_role::ContributorRole;
pub use db_id::DbId;
pub use device_type::DeviceType;
pub use disk_path::DiskPath;
pub use format_metadata_schema::FormatMetadataSchema;
pub use identifier::{Identifier, IdentifierParseError};
pub use identifier_kind::IdentifierKind;
pub use isbn::{Isbn, IsbnError};
pub use known::{CommonLanguages, KnownFormats, KnownGenres, KnownReadingSources, KnownSubjects};
pub use pairing_status::PairingStatus;
pub use progress_unit::{ProgressUnit, Progression};
pub use published_date::{PublishedDate, PublishedDateError};
pub use reading_length::progression_to_normalized;
#[cfg(feature = "search")]
pub use search::__field_exists_alias;
#[cfg(feature = "search")]
pub use search::__identifier_kind_alias;
#[cfg(feature = "search")]
pub use search::__identifier_search_alias;
#[cfg(feature = "search")]
pub use search::__isbn_alias;
#[cfg(feature = "search")]
pub use search::__must_not_alias;
/// Re-exported so `identifier_search_enum!` / `identifier_search_to_ast!` macros can
/// call `$crate::__paste_use::paste!` to generate `By<Kind>` / `Has<Kind>` enum variants.
#[cfg(feature = "search")]
pub use search::__paste_use;
#[cfg(feature = "search")]
pub use search::__user_input_ast_alias;
pub use series_type::SeriesType;
pub use urn::{Urn, UrnParseError};
pub use work_filters::{SortDirection, SortField, SortSpec, WorkFilters, WorkSortBy};
pub use work_status::WorkStatus;

pub fn now_primitive() -> time::PrimitiveDateTime {
    time::PrimitiveDateTime::new(
        time::OffsetDateTime::now_utc().date(),
        time::OffsetDateTime::now_utc().time(),
    )
}

pub const KNOWN_FORMAT_TIME_MS: u64 = 1_778_544_000_000;
pub const KNOWN_LANGUAGE_TIME_MS: u64 = KNOWN_FORMAT_TIME_MS + 1;
pub const KNOWN_GENRE_TIME_MS: u64 = KNOWN_LANGUAGE_TIME_MS + 1;
pub const DEVICE_TYPE_TIME_MS: u64 = KNOWN_GENRE_TIME_MS + 1;
pub const PAIRING_STATUS_TIME_MS: u64 = DEVICE_TYPE_TIME_MS + 1;
pub const KNOWN_SUBJECT_TIME_MS: u64 = PAIRING_STATUS_TIME_MS + 1;
pub const BOOK_CONDITION_TIME_MS: u64 = KNOWN_SUBJECT_TIME_MS + 1;
pub const KNOWN_READING_SOURCE_TIME_MS: u64 = BOOK_CONDITION_TIME_MS + 1;
