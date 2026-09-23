use core::default::Default;
use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use specta::{Type, datatype::DataType};
use ulid::Ulid;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct DbId(pub Ulid);

impl Type for DbId {
    fn definition(types: &mut specta::Types) -> DataType {
        // ULIDs are serialized as strings, so expose them as strings in the type system.
        <str as Type>::definition(types)
    }
}

impl DbId {
    pub fn new() -> Self {
        Self(Ulid::generate())
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Ulid::from_bytes(bytes))
    }

    pub fn to_bytes(self) -> [u8; 16] {
        self.0.to_bytes()
    }
    /// Lowercase hex encoding of the 16-byte ID (32 hex chars).
    pub fn to_hex(self) -> String {
        hex::encode(self.0.to_bytes())
    }

    /// Parse a 32-char hex string (lowercase or uppercase) into a DbId.
    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        let bytes = hex::decode(s.as_bytes())?;
        let arr: [u8; 16] = bytes
            .try_into()
            .map_err(|_| hex::FromHexError::InvalidStringLength)?;
        Ok(DbId::from_bytes(arr))
    }
}

#[cfg(feature = "uniffi")]
uniffi::custom_type!(DbId, String, {
    lower: |id| id.to_string(),
    try_lift: |s| Ok(s.parse::<DbId>()?),
});

impl std::ops::Deref for DbId {
    type Target = Ulid;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for DbId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for DbId {
    type Err = ulid::DecodeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ulid::from_str(s).map(Self)
    }
}

impl Serialize for DbId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for DbId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ulid::from_str(&s)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

impl From<DbId> for sea_orm::Value {
    fn from(val: DbId) -> Self {
        sea_orm::Value::Bytes(Some(val.to_bytes().to_vec()))
    }
}

impl sea_orm::sea_query::ValueType for DbId {
    fn try_from(v: sea_orm::Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            sea_orm::Value::Bytes(Some(bytes)) => {
                let arr: [u8; 16] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| sea_orm::sea_query::ValueTypeErr)?;
                Ok(DbId::from_bytes(arr))
            }
            sea_orm::Value::String(Some(s)) => Ulid::from_str(&s)
                .map(DbId)
                .map_err(|_| sea_orm::sea_query::ValueTypeErr),
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        "DbId".to_owned()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::Bytes
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::Binary(16)
    }
}

impl sea_orm::TryGetable for DbId {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &sea_orm::QueryResult,
        index: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        // Try BINARY(16) bytes first.
        // Preserve the first error so Null propagates correctly to Option<DbId>.
        let bytes_result = <Vec<u8> as sea_orm::TryGetable>::try_get_by(res, index);
        if let Ok(v) = bytes_result {
            let arr: [u8; 16] = v.as_slice().try_into().map_err(|_| {
                sea_orm::TryGetError::DbErr(sea_orm::DbErr::Custom(format!(
                    "Invalid DbId: expected 16 bytes, got {} bytes",
                    v.len()
                )))
            })?;
            return Ok(DbId::from_bytes(arr));
        }

        // Fallback: try String -- ULID base32 or hex.
        match <String as sea_orm::TryGetable>::try_get_by(res, index) {
            Ok(s) => {
                // Try ULID string (26 chars, Crockford base32).
                if let Ok(ulid) = Ulid::from_str(&s) {
                    return Ok(DbId(ulid));
                }
                // Try hex string (32 chars, "017f21f0...").
                if let Ok(id) = DbId::from_hex(&s) {
                    return Ok(id);
                }
                // String value wasn't parseable as DbId.
                Err(sea_orm::TryGetError::DbErr(sea_orm::DbErr::Custom(
                    format!(
                        "Invalid DbId: string value {:?} is not a ULID or hex string",
                        &s[..s.len().min(64)],
                    ),
                )))
            }
            // Propagate Null (and column-not-found) so Option<DbId> sees it.
            Err(e) => Err(e),
        }
    }
}

impl sea_orm::sea_query::Nullable for DbId {
    fn null() -> sea_orm::Value {
        sea_orm::Value::Bytes(None)
    }
}

impl sea_orm::TryFromU64 for DbId {
    fn try_from_u64(_: u64) -> Result<Self, sea_orm::DbErr> {
        Err(sea_orm::DbErr::ConvertFromU64("DbId"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dbid_roundtrip() {
        let id = DbId::new();
        let bytes = id.to_bytes();
        let restored = DbId::from_bytes(bytes);
        assert_eq!(id, restored);
    }

    #[test]
    fn test_dbid_display() {
        let id = DbId::new();
        let s = id.to_string();
        assert_eq!(s.len(), 26);
    }

    #[test]
    fn test_dbid_from_str() {
        let id = DbId::new();
        let s = id.to_string();
        let parsed: DbId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_dbid_hex_roundtrip() {
        let id = DbId::new();
        let hex = id.to_hex();
        assert_eq!(hex.len(), 32);
        let parsed = DbId::from_hex(&hex).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_dbid_from_hex_rejects_wrong_length() {
        assert!(DbId::from_hex("abcd").is_err());
        assert!(DbId::from_hex("").is_err());
    }

    #[test]
    fn test_dbid_from_hex_rejects_bad_chars() {
        let bad = "zz".repeat(16);
        assert!(DbId::from_hex(&bad).is_err());
    }
}

#[cfg(all(test, feature = "uniffi"))]
mod uniffi_tests {
    use super::*;
    use crate::UniFfiTag;
    use uniffi::FfiConverter;

    #[test]
    fn string_round_trip() {
        let id = DbId::new();
        let buf = <DbId as FfiConverter<UniFfiTag>>::lower(id);
        let lifted =
            <DbId as FfiConverter<UniFfiTag>>::try_lift(buf).expect("lowering is always valid");
        assert_eq!(lifted, id);
    }

    #[test]
    fn wire_format_is_the_ulid_string() {
        let ulid = Ulid::from_string("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
        let buf = <DbId as FfiConverter<UniFfiTag>>::lower(DbId(ulid));
        let s = <String as FfiConverter<UniFfiTag>>::try_lift(buf).unwrap();
        assert_eq!(s, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
    }

    #[test]
    fn lift_rejects_garbage() {
        let buf = <String as FfiConverter<UniFfiTag>>::lower("not-a-ulid".to_string());
        assert!(<DbId as FfiConverter<UniFfiTag>>::try_lift(buf).is_err());
    }
}
