// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};
use std::{error, fmt, str::FromStr};

/// Represents a directory hash.
///
/// A directory hash is typically stable, but it can change over time.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub struct DirectoryHash {
    numeric: u64,
}

impl DirectoryHash {
    /// The width of the hex representation of a directory hash.
    pub const WIDTH: usize = std::mem::size_of::<u64>() * 2;

    /// Creates a new `DirectoryHash` from its numeric representation.
    #[inline]
    pub fn new(numeric: u64) -> Self {
        Self { numeric }
    }

    /// Returns the numeric representation of the hash.
    #[inline]
    pub fn numeric(&self) -> u64 {
        self.numeric
    }
}

impl fmt::Display for DirectoryHash {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Print out the bytes as lowercase hex, with leading zeroes.
        write!(f, "{:0width$x}", self.numeric, width = Self::WIDTH)
    }
}

impl FromStr for DirectoryHash {
    type Err = ParseDirectoryHashError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Only accept lowercase hashes of the right length.
        if s.len() != Self::WIDTH {
            return Err(ParseDirectoryHashError {
                input: s.into(),
                err: format!("length {} is not {}", s.len(), Self::WIDTH),
            });
        }
        if !s
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f'))
        {
            return Err(ParseDirectoryHashError {
                input: s.into(),
                err: "input not in [0-9a-f]".to_string(),
            });
        }

        let numeric = u64::from_str_radix(s, 16).map_err(|err| ParseDirectoryHashError {
            input: s.into(),
            err: format!("{}", err),
        })?;
        Ok(Self::new(numeric))
    }
}

impl Serialize for DirectoryHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DirectoryHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(D::Error::custom)
    }
}

#[cfg(feature = "rusqlite")]
mod rusqlite_impls {
    use super::*;
    use rusqlite::{
        types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
        ToSql,
    };

    impl FromSql for DirectoryHash {
        fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
            value.as_blob().and_then(|v| {
                // TODO: better error message
                let v: [u8; 8] = v
                    .try_into()
                    .map_err(|err| FromSqlError::Other(Box::new(err)))?;
                Ok(DirectoryHash::new(u64::from_be_bytes(v)))
            })
        }
    }

    impl ToSql for DirectoryHash {
        #[inline]
        fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
            Ok(self.numeric.to_be_bytes().to_vec().into())
        }
    }
}

/// An error encountered while parsing a directory hash.
#[derive(Clone, Debug)]
pub struct ParseDirectoryHashError {
    input: String,
    err: String,
}

impl fmt::Display for ParseDirectoryHashError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "could not parse directory hash '{}': {}",
            self.input, self.err
        )
    }
}

impl error::Error for ParseDirectoryHashError {}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn directory_hash_basic() {
        const HASH_NUMERIC: u64 = 0x01234567;
        const HASH_STR: &str = "0000000001234567";
        let hash = DirectoryHash::new(HASH_NUMERIC);
        assert_eq!(hash.to_string(), HASH_STR);
        assert_eq!(hash.numeric(), HASH_NUMERIC);

        let serialized = serde_json::to_string(&hash).expect("serialization succeeded");
        assert_eq!(serialized, format!("\"{}\"", HASH_STR));

        let deserialized: DirectoryHash =
            serde_json::from_str(&serialized).expect("deserialization succeeded");
        assert_eq!(deserialized, hash);
    }

    proptest! {
        #[test]
        fn directory_hash_to_string_roundtrip(numeric: u64) {
            let hash = DirectoryHash::new(numeric);
            let to_string = hash.to_string();
            let roundtrip = to_string.parse().expect("to_string roundtrip succeeded");
            assert_eq!(hash, roundtrip, "hash matches display roundtrip");
        }

        #[test]
        fn directory_hash_serde_roundtrip(numeric: u64) {
            let hash = DirectoryHash::new(numeric);

            let serialized = serde_json::to_string(&hash).expect("serialization succeeded");
            let deserialized: DirectoryHash =
                serde_json::from_str(&serialized).expect("deserialization succeeded");
            assert_eq!(deserialized, hash, "hash matches serde roundtrip");
        }

        #[cfg(feature = "rusqlite")]
        #[test]
        fn directory_hash_rusqlite_roundtrip(numeric: u64) {
            use rusqlite::Connection;

            let conn = Connection::open_in_memory().expect("in-memory DB succeeded");
            conn.execute("CREATE TABLE directory_hash (hash BLOB)", [])
                .expect("creating table succeeded");

            let hash = DirectoryHash::new(numeric);
            conn.execute("INSERT INTO directory_hash (hash) VALUES (?1)", [hash])
                .expect("insert succeeded");

            let roundtrip: DirectoryHash = conn
                .query_row("SELECT (hash) from directory_hash", [], |row| {
                    row.get("hash")
                })
                .expect("select succeeded");
            assert_eq!(hash, roundtrip, "hash through SQL roundtrip matches");
        }
    }
}
