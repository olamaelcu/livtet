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
}
