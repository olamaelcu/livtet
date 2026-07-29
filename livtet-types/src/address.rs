use std::{
    fmt::{Display, Formatter},
    net::SocketAddr,
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use specta::Type;

/// A network endpoint as `host:port`, e.g. `"0.0.0.0:3121"`.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Type)]
pub struct Address(pub SocketAddr);

impl Address {
    pub fn new(addr: SocketAddr) -> Self {
        Self(addr)
    }

    pub fn into_inner(self) -> SocketAddr {
        self.0
    }
}

impl std::ops::Deref for Address {
    type Target = SocketAddr;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for Address {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for Address {
    type Err = <SocketAddr as FromStr>::Err;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        SocketAddr::from_str(s).map(Self)
    }
}

impl Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        SocketAddr::from_str(&s)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

impl From<Address> for sea_orm::Value {
    fn from(val: Address) -> Self {
        sea_orm::Value::String(Some(val.0.to_string()))
    }
}

impl sea_orm::sea_query::ValueType for Address {
    fn try_from(v: sea_orm::Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            sea_orm::Value::String(Some(s)) => SocketAddr::from_str(s.as_str())
                .map(Address)
                .map_err(|_| sea_orm::sea_query::ValueTypeErr),
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        "Address".to_owned()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::String
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::Text
    }
}

impl sea_orm::TryGetable for Address {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &sea_orm::QueryResult,
        index: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        <String as sea_orm::TryGetable>::try_get_by(res, index).and_then(|s| {
            SocketAddr::from_str(&s).map(Address).map_err(|_| {
                sea_orm::TryGetError::DbErr(sea_orm::DbErr::Custom(format!("Invalid Address: {s}")))
            })
        })
    }
}

impl sea_orm::sea_query::Nullable for Address {
    fn null() -> sea_orm::Value {
        sea_orm::Value::String(None)
    }
}

impl sea_orm::TryFromU64 for Address {
    fn try_from_u64(_: u64) -> Result<Self, sea_orm::DbErr> {
        Err(sea_orm::DbErr::ConvertFromU64("Address"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_valid_ipv4() {
        let a: Address = "0.0.0.0:3121".parse().unwrap();
        assert_eq!(a.ip().to_string(), "0.0.0.0");
        assert_eq!(a.port(), 3121);
    }

    #[test]
    fn from_str_valid_ipv6() {
        let a: Address = "[::1]:8080".parse().unwrap();
        assert_eq!(a.ip().to_string(), "::1");
        assert_eq!(a.port(), 8080);
    }

    #[test]
    fn from_str_invalid_missing_port() {
        let r = "0.0.0.0".parse::<Address>();
        assert!(r.is_err());
    }

    #[test]
    fn from_str_invalid_bad_ip() {
        let r = "not-an-address:80".parse::<Address>();
        assert!(r.is_err());
    }

    #[test]
    fn display_roundtrip() {
        let original = Address("127.0.0.1:9000".parse().unwrap());
        let s = original.to_string();
        let parsed: Address = s.parse().unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn serde_roundtrip() {
        let original = Address("10.0.0.42:65535".parse().unwrap());
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"10.0.0.42:65535\"");
        let parsed: Address = serde_json::from_str(&json).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn sea_orm_value_to_from() {
        let a = Address("192.168.1.1:8080".parse().unwrap());
        let v: sea_orm::Value = a.clone().into();
        match &v {
            sea_orm::Value::String(Some(s)) => assert_eq!(s.as_str(), "192.168.1.1:8080"),
            _ => panic!("expected Value::String(Some(_))"),
        }
        let back = <Address as sea_orm::sea_query::ValueType>::try_from(v).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn nullable_returns_null_string() {
        assert_eq!(
            <Address as sea_orm::sea_query::Nullable>::null(),
            sea_orm::Value::String(None)
        );
    }

    #[test]
    fn specta_type_maps_to_string() {
        use specta::datatype::{
            DataType, Fields, NamedReferenceType, Primitive, Reference, inline,
        };

        let inline_dt = inline(&mut Default::default(), |t| {
            <Address as specta::Type>::definition(t)
        });

        fn contains_str_primitive(dt: &DataType) -> bool {
            match dt {
                DataType::Primitive(Primitive::str) => true,
                DataType::Struct(s) => match &s.fields {
                    Fields::Unnamed(u) => u
                        .fields
                        .iter()
                        .any(|f| f.ty.as_ref().is_some_and(contains_str_primitive)),
                    Fields::Named(n) => n
                        .fields
                        .iter()
                        .any(|(_, f)| f.ty.as_ref().is_some_and(contains_str_primitive)),
                    Fields::Unit => false,
                },
                DataType::Nullable(inner) => contains_str_primitive(inner),
                DataType::Reference(Reference::Named(nr)) => match &nr.inner {
                    NamedReferenceType::Inline { dt, .. } => contains_str_primitive(dt),
                    NamedReferenceType::Reference { .. } | NamedReferenceType::Recursive(_) => {
                        false
                    }
                },
                _ => false,
            }
        }

        assert!(
            contains_str_primitive(&inline_dt),
            "Address should resolve through to a string primitive when inlined, got {inline_dt:?}"
        );
    }
}
