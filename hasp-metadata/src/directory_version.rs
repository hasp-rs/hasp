// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use either::Either;
use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};
use std::{error, fmt, str::FromStr};

/// Represents a directory version.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DirectoryVersion {
    /// A semantic version. Can have semantic ranges applied to it.
    Semantic(semver::Version),

    /// A literal version, compared exactly.
    ///
    /// This can be any sort of arbitrary byte sequence.
    Literal(String),
}

impl DirectoryVersion {
    /// The prefix used while serializing semantic versions.
    pub const SEM_PREFIX: &'static str = "sem:";

    /// The prefix used while serializing literal versions.
    pub const LIT_PREFIX: &'static str = "lit:";

    /// Creates a new semantic version.
    #[inline]
    pub fn new_semantic(version: semver::Version) -> Self {
        DirectoryVersion::Semantic(version)
    }

    /// Creates a new literal version.
    #[inline]
    pub fn new_literal(version: impl Into<String>) -> Self {
        DirectoryVersion::Literal(version.into())
    }

    /// Returns the semantic version.
    pub fn as_semantic(&self) -> Option<&semver::Version> {
        match self {
            DirectoryVersion::Semantic(v) => Some(v),
            _ => None,
        }
    }

    /// Returns the literal version.
    pub fn as_literal(&self) -> Option<&str> {
        match self {
            DirectoryVersion::Literal(v) => Some(v),
            _ => None,
        }
    }
}

impl fmt::Display for DirectoryVersion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DirectoryVersion::Semantic(version) => write!(f, "{}{}", Self::SEM_PREFIX, version),
            DirectoryVersion::Literal(version) => write!(f, "{}{}", Self::LIT_PREFIX, version),
        }
    }
}

impl FromStr for DirectoryVersion {
    type Err = ParseDirectoryVersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(v) = s.strip_prefix("sem:") {
            let version: semver::Version = v.parse().map_err(|err| ParseDirectoryVersionError {
                input: s.into(),
                err: Either::Left(err),
            })?;
            Ok(DirectoryVersion::Semantic(version))
        } else if let Some(v) = s.strip_prefix("lit:") {
            Ok(DirectoryVersion::Literal(v.into()))
        } else {
            Err(ParseDirectoryVersionError {
                input: s.into(),
                err: Either::Right("input begins with neither 'sem:' nor 'lit:'"),
            })
        }
    }
}

impl Serialize for DirectoryVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DirectoryVersion {
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

    impl FromSql for DirectoryVersion {
        fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
            value
                .as_str()
                .and_then(|v| v.parse().map_err(|err| FromSqlError::Other(Box::new(err))))
        }
    }

    impl ToSql for DirectoryVersion {
        #[inline]
        fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
            Ok(self.to_string().into())
        }
    }
}

/// An error encountered while parsing a directory version.
#[derive(Debug)]
pub struct ParseDirectoryVersionError {
    input: String,
    err: Either<semver::Error, &'static str>,
}

impl fmt::Display for ParseDirectoryVersionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "could not parse directory version '{}'", self.input)?;
        if let Either::Right(s) = &self.err {
            write!(f, ": {}", s)?;
        };
        Ok(())
    }
}

impl error::Error for ParseDirectoryVersionError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.err {
            Either::Left(err) => Some(err),
            Either::Right(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use semver::{BuildMetadata, Prerelease};

    #[test]
    fn directory_version_basic() {
        let mut sem_version = semver::Version::new(2, 3, 4);
        sem_version.pre = Prerelease::new("beta.0").expect("prerelease parsed");
        const SEM_VERSION_STR: &str = "sem:2.3.4-beta.0";

        let version = DirectoryVersion::new_semantic(sem_version);
        assert_eq!(version.to_string(), SEM_VERSION_STR);

        let serialized = serde_json::to_string(&version).expect("serialization succeeded");
        assert_eq!(serialized, format!("\"{}\"", SEM_VERSION_STR));

        let deserialized: DirectoryVersion =
            serde_json::from_str(&serialized).expect("deserialization succeeded");
        assert_eq!(deserialized, version);

        // TODO: also test literal versions
    }

    impl Arbitrary for DirectoryVersion {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            // Can't have leading zeros in the prerelease identifier.
            const VERSION_REGEX: &str = "([a-zA-Z1-9]+\\.)*[a-zA-Z1-9]+";

            let major_minor_patch = (any::<u64>(), any::<u64>(), any::<u64>());
            let pre_strategy = prop_oneof![3 => VERSION_REGEX, 1 => Just("".to_owned())];
            let build_strategy = prop_oneof![3 => VERSION_REGEX, 1 => Just("".to_owned())];
            let semver_strategy = (major_minor_patch, pre_strategy, build_strategy).prop_map(
                |((major, minor, patch), pre, build)| {
                    let pre = Prerelease::new(&pre)
                        .unwrap_or_else(|err| panic!("prerelease is valid: {}: {}", pre, err));
                    let build = BuildMetadata::new(&build).expect("build metadata is valid");
                    DirectoryVersion::new_semantic(semver::Version {
                        major,
                        minor,
                        patch,
                        pre,
                        build,
                    })
                },
            );

            prop_oneof![
                VERSION_REGEX.prop_map(DirectoryVersion::Literal),
                semver_strategy
            ]
            .boxed()
        }
    }

    proptest! {
        #[test]
        fn directory_version_to_string_roundtrip(version: DirectoryVersion) {
            let to_string = version.to_string();
            let roundtrip = to_string.parse().expect("to_string roundtrip succeeded");
            assert_eq!(version, roundtrip, "version matches display roundtrip");
        }

        #[test]
        fn directory_version_serde_roundtrip(version: DirectoryVersion) {
            let serialized = serde_json::to_string(&version).expect("serialization succeeded");
            let deserialized: DirectoryVersion =
                serde_json::from_str(&serialized).expect("deserialization succeeded");
            assert_eq!(deserialized, version, "version matches serde roundtrip");
        }

        #[cfg(feature = "rusqlite")]
        #[test]
        fn directory_version_rusqlite_roundtrip(version: DirectoryVersion) {
            use rusqlite::Connection;

            let conn = Connection::open_in_memory().expect("in-memory DB succeeded");
            conn.execute("CREATE TABLE directory_version (version TEXT)", [])
                .expect("creating table succeeded");

            conn.execute("INSERT INTO directory_version (version) VALUES (?1)", [&version])
                .expect("insert succeeded");

            let roundtrip: DirectoryVersion = conn
                .query_row("SELECT (version) from directory_version", [], |row| {
                    row.get("version")
                })
                .expect("select succeeded");
            assert_eq!(version, roundtrip, "version through SQL roundtrip matches");
        }
    }
}
