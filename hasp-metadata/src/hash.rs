// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

// This can't be done as a trait because you can't implement a foreign trait like that, even for
// a sealed local trait.

use hex::ToHex;
use std::{error, fmt};

macro_rules! hash_impls {
    ($t: ty, $mod_name: ident) => {
        mod $mod_name {
            use super::*;
            use hex::{FromHex, ToHex};
            use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};
            use std::{fmt, str::FromStr};

            impl fmt::Display for $t {
                #[inline]
                fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    // Print out the bytes as lowercase hex, with leading zeroes.
                    write!(
                        f,
                        "{:0width$}",
                        self.to_be_bytes().encode_hex::<String>(),
                        width = Self::BYTES * 2
                    )
                }
            }

            impl FromStr for $t {
                type Err = $crate::ParseHashError;

                fn from_str(s: &str) -> Result<Self, Self::Err> {
                    // Only accept lowercase hashes of the right length.
                    if s.len() != Self::BYTES * 2 {
                        return Err($crate::ParseHashError {
                            description: Self::DESCRIPTION,
                            input: s.into(),
                            err: format!("length {} is not {}", s.len(), Self::BYTES * 2),
                        });
                    }
                    if !s
                        .chars()
                        .all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f'))
                    {
                        return Err($crate::ParseHashError {
                            description: Self::DESCRIPTION,
                            input: s.into(),
                            err: "input not in [0-9a-f]".to_string(),
                        });
                    }

                    let bytes = <[u8; Self::BYTES]>::from_hex(s).expect("already checked validity");
                    Ok(Self::from_be_bytes(bytes))
                }
            }

            impl Serialize for $t {
                fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                where
                    S: Serializer,
                {
                    self.to_string().serialize(serializer)
                }
            }

            impl<'de> Deserialize<'de> for $t {
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

                impl FromSql for $t {
                    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                        value.as_blob().and_then(|input| {
                            let bytes: [u8; Self::BYTES] = input.try_into().map_err(|_| {
                                let err = $crate::ParseHashError::from_blob(
                                    Self::DESCRIPTION,
                                    input,
                                    format!("blob length {} is not {}", input.len(), Self::BYTES),
                                );
                                FromSqlError::Other(Box::new(err))
                            })?;
                            Ok(Self::from_be_bytes(bytes))
                        })
                    }
                }

                impl ToSql for $t {
                    #[inline]
                    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                        Ok(self.to_be_bytes().to_vec().into())
                    }
                }
            }
        }
    };
}

/// An error encountered while parsing a hash from a string or SQL blob.
#[derive(Clone, Debug)]
pub struct ParseHashError {
    pub(crate) description: &'static str,
    pub(crate) input: String,
    pub(crate) err: String,
}

impl ParseHashError {
    pub fn description(&self) -> &'static str {
        self.description
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn error_string(&self) -> &str {
        &self.err
    }

    pub(crate) fn from_blob(
        description: &'static str,
        input: &[u8],
        err: impl Into<String>,
    ) -> Self {
        Self {
            description,
            input: input.encode_hex::<String>(),
            err: err.into(),
        }
    }
}

impl fmt::Display for ParseHashError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "could not parse {} '{}': {}",
            self.description, self.input, self.err
        )
    }
}

impl error::Error for ParseHashError {}
