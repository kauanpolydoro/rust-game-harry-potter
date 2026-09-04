use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

const MAX_ID_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CatalogId(String);

impl CatalogId {
    /// Parses a stable, locale-independent catalog identifier.
    ///
    /// # Errors
    ///
    /// Returns an error when the identifier is not a lowercase namespace and
    /// slug separated by one colon.
    pub fn parse(value: &str) -> Result<Self, InvalidId> {
        parse_namespaced_id(value).map(Self)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CatalogId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for CatalogId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CatalogId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CardInstanceId(String);

impl CardInstanceId {
    /// Parses an opaque runtime card instance identifier.
    ///
    /// # Errors
    ///
    /// Returns an error when the identifier is empty, too long or not namespaced.
    pub fn parse(value: &str) -> Result<Self, InvalidId> {
        parse_namespaced_id(value).map(Self)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleId(String);

impl RuleId {
    /// Parses a stable rule identifier.
    ///
    /// # Errors
    ///
    /// Returns an error when the identifier is not a lowercase namespace and
    /// slug separated by one colon.
    pub fn parse(value: &str) -> Result<Self, InvalidId> {
        parse_namespaced_id(value).map(Self)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for RuleId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RuleId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidId;

impl fmt::Display for InvalidId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "ID must be at most 256 bytes and contain a lowercase namespace and slug separated by one colon",
        )
    }
}

impl std::error::Error for InvalidId {}

fn parse_namespaced_id(value: &str) -> Result<String, InvalidId> {
    let mut parts = value.split(':');
    let namespace = parts.next().ok_or(InvalidId)?;
    let slug = parts.next().ok_or(InvalidId)?;
    let valid_part = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte)
            })
    };

    if value.len() > MAX_ID_BYTES
        || parts.next().is_some()
        || !valid_part(namespace)
        || !valid_part(slug)
    {
        return Err(InvalidId);
    }

    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_accept_256_bytes_and_reject_257_bytes() {
        let accepted = format!("a:{}", "b".repeat(254));
        let rejected = format!("a:{}", "b".repeat(255));

        assert_eq!(accepted.len(), MAX_ID_BYTES);
        assert_eq!(rejected.len(), MAX_ID_BYTES + 1);
        assert!(CatalogId::parse(&accepted).is_ok());
        assert!(CardInstanceId::parse(&accepted).is_ok());
        assert!(RuleId::parse(&accepted).is_ok());
        assert!(CatalogId::parse(&rejected).is_err());
        assert!(CardInstanceId::parse(&rejected).is_err());
        assert!(RuleId::parse(&rejected).is_err());
    }
}
