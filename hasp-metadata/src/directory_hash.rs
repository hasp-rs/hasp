// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Represents a directory hash.
///
/// A directory hash is typically stable, but it can change over time.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub struct DirectoryHash {
    numeric: u64,
}

impl DirectoryHash {
    /// The width of a directory hash, in bytes.
    pub const BYTES: usize = std::mem::size_of::<u64>();

    /// Creates a new `DirectoryHash` from its numeric representation.
    #[inline]
    pub fn new(numeric: u64) -> Self {
        Self { numeric }
    }

    /// Creates a new `DirectoryHash` from big-endian bytes.
    #[inline]
    pub fn from_be_bytes(bytes: [u8; Self::BYTES]) -> Self {
        Self {
            numeric: u64::from_be_bytes(bytes),
        }
    }

    /// Returns a big-endian representation.
    #[inline]
    pub fn to_be_bytes(&self) -> [u8; Self::BYTES] {
        self.numeric.to_be_bytes()
    }

    /// Returns the numeric representation of the hash.
    #[inline]
    pub fn numeric(&self) -> u64 {
        self.numeric
    }

    const DESCRIPTION: &'static str = "directory hash";
}

hash_impls!(DirectoryHash, directory_hash);

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
