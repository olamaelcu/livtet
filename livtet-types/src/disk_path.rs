use core::default::Default;
use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct DiskPath(Utf8PathBuf);

impl DiskPath {
    pub fn new() -> Self {
        Self(Utf8PathBuf::new())
    }

    pub fn from_path(path: impl AsRef<camino::Utf8Path>) -> Self {
        Self(path.as_ref().to_path_buf())
    }

    pub fn as_path(&self) -> &camino::Utf8Path {
        &self.0
    }

    pub fn into_path_buf(self) -> Utf8PathBuf {
        self.0
    }
}

#[cfg(feature = "uniffi")]
uniffi::custom_type!(DiskPath, String, {
    lower: |p| p.to_string(),
    try_lift: |s: String| Ok(DiskPath::from_path(camino::Utf8Path::new(&s))),
});

impl std::ops::Deref for DiskPath {
    type Target = camino::Utf8Path;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for DiskPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for DiskPath {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Utf8PathBuf::from(s)))
    }
}

impl From<Utf8PathBuf> for DiskPath {
    fn from(path: Utf8PathBuf) -> Self {
        Self(path)
    }
}

impl Serialize for DiskPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for DiskPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Self(Utf8PathBuf::from(s)))
    }
}

impl From<DiskPath> for sea_orm::Value {
    fn from(val: DiskPath) -> Self {
        sea_orm::Value::String(Some(val.0.into_string()))
    }
}

impl sea_orm::sea_query::ValueType for DiskPath {
    fn try_from(v: sea_orm::Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            sea_orm::Value::String(Some(s)) => Ok(Self(Utf8PathBuf::from(s.as_str()))),
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        "DiskPath".to_owned()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::String
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::Text
    }
}

impl sea_orm::TryGetable for DiskPath {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &sea_orm::QueryResult,
        index: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        <String as sea_orm::TryGetable>::try_get_by(res, index).map(|s| Self(Utf8PathBuf::from(s)))
    }
}

impl sea_orm::sea_query::Nullable for DiskPath {
    fn null() -> sea_orm::Value {
        sea_orm::Value::String(None)
    }
}

impl sea_orm::TryFromU64 for DiskPath {
    fn try_from_u64(_: u64) -> Result<Self, sea_orm::DbErr> {
        Err(sea_orm::DbErr::Custom(
            "DiskPath cannot be created from u64".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diskpath_roundtrip() {
        let path = DiskPath::from_path("/tmp/test/path");
        let s = path.to_string();
        let restored: DiskPath = s.parse().unwrap();
        assert_eq!(path, restored);
    }

    #[test]
    fn test_diskpath_display() {
        let path = DiskPath::from_path("/tmp/test");
        assert_eq!(path.to_string(), "/tmp/test");
    }

    #[test]
    fn test_diskpath_from_str() {
        let path = DiskPath::from_path("/tmp/test");
        let s = path.to_string();
        let parsed: DiskPath = s.parse().unwrap();
        assert_eq!(path, parsed);
    }

    #[test]
    fn constructors_and_accessors_agree_on_the_inner_path() {
        assert_eq!(DiskPath::new(), DiskPath::default());
        assert!(DiskPath::new().as_path().as_str().is_empty());

        let path = camino::Utf8Path::new("/books/a.epub");
        let dp = DiskPath::from_path(path);
        assert_eq!(dp.as_path(), path);
        assert_eq!(dp.clone().into_path_buf(), path.to_path_buf());
        assert_eq!(DiskPath::from(path.to_path_buf()), dp);
        // Deref reaches Utf8Path's own API.
        assert_eq!(dp.extension(), Some("epub"));
    }

    #[test]
    fn serde_uses_the_bare_path_string() {
        let dp = DiskPath::from_path(camino::Utf8Path::new("/books/a.epub"));
        let json = serde_json::to_string(&dp).expect("serialize");
        assert_eq!(json, "\"/books/a.epub\"");
        assert_eq!(
            serde_json::from_str::<DiskPath>(&json).expect("deserialize"),
            dp
        );
    }

    #[test]
    fn value_round_trips_as_text() {
        use sea_orm::Value;
        use sea_orm::sea_query::ValueType;
        let dp = DiskPath::from_path(camino::Utf8Path::new("/books/a.epub"));
        assert_eq!(
            Value::from(dp.clone()),
            Value::String(Some("/books/a.epub".to_string()))
        );
        assert_eq!(
            <DiskPath as ValueType>::try_from(Value::from(dp.clone())).expect("round trip"),
            dp
        );
    }

    #[test]
    fn value_type_rejects_non_string_variants() {
        use sea_orm::Value;
        use sea_orm::sea_query::ValueType;
        for value in [
            Value::BigInt(Some(1)),
            Value::String(None),
            Value::Bool(Some(true)),
        ] {
            assert!(
                <DiskPath as ValueType>::try_from(value.clone()).is_err(),
                "{value:?} must not convert"
            );
        }
    }

    #[test]
    fn value_type_metadata_describes_a_text_column() {
        use sea_orm::Value;
        use sea_orm::sea_query::{ArrayType, ColumnType, Nullable, ValueType};
        assert_eq!(<DiskPath as ValueType>::type_name(), "DiskPath");
        assert_eq!(<DiskPath as ValueType>::array_type(), ArrayType::String);
        assert_eq!(<DiskPath as ValueType>::column_type(), ColumnType::Text);
        assert_eq!(<DiskPath as Nullable>::null(), Value::String(None));
    }

    /// A path has no integer identity; the impl exists only for sea-orm.
    #[test]
    fn cannot_be_built_from_a_u64() {
        use sea_orm::TryFromU64;
        assert!(DiskPath::try_from_u64(1).is_err());
    }
}

#[cfg(all(test, feature = "uniffi"))]
mod uniffi_tests {
    use super::*;
    use crate::UniFfiTag;
    use uniffi::FfiConverter;

    #[test]
    fn string_round_trip() {
        let path = DiskPath::from_path(camino::Utf8Path::new("/data/books/some.epub"));
        let buf = <DiskPath as FfiConverter<UniFfiTag>>::lower(path.clone());
        let lifted = <DiskPath as FfiConverter<UniFfiTag>>::try_lift(buf).unwrap();
        assert_eq!(lifted, path);
    }

    #[test]
    fn wire_format_is_the_path_string() {
        let path = DiskPath::from_path(camino::Utf8Path::new("/data/books/some.epub"));
        let buf = <DiskPath as FfiConverter<UniFfiTag>>::lower(path);
        let s = <String as FfiConverter<UniFfiTag>>::try_lift(buf).unwrap();
        assert_eq!(s, "/data/books/some.epub");
    }
}
