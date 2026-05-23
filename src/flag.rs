//! Flags set associated with an m2dir entry.

use core::fmt;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use std::collections::HashSet;

/// Set of flags attached to an m2dir entry.
///
/// Each flag is an arbitrary UTF-8 string; serialization to the
/// `.meta/<id>.flags` sidecar is one flag per line.
#[derive(Clone, Debug, Default)]
pub struct Flags(HashSet<String>);

impl Flags {
    /// Returns an iterator over the flags in this set.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    /// Returns the number of flags in this set.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the set contains no flags.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Inserts a flag into the set. Returns `true` if it was not
    /// already present.
    pub fn insert(&mut self, flag: impl Into<String>) -> bool {
        self.0.insert(flag.into())
    }

    /// Removes a flag from the set. Returns `true` if it was present.
    pub fn remove(&mut self, flag: &str) -> bool {
        self.0.remove(flag)
    }

    /// Returns `true` if the set contains the given flag.
    pub fn contains(&self, flag: &str) -> bool {
        self.0.contains(flag)
    }

    /// Adds every flag from `flags` to this set.
    pub fn extend(&mut self, flags: Flags) {
        self.0.extend(flags.0);
    }

    /// Removes every flag in `flags` from this set.
    pub fn difference(&mut self, flags: &Flags) {
        self.0 = self.0.difference(&flags.0).cloned().collect();
    }

    /// Serializes the flag set to its on-disk `.flags` representation:
    /// one flag per line, deterministic alphabetical order.
    pub fn to_sidecar(&self) -> String {
        let mut sorted: Vec<&str> = self.0.iter().map(String::as_str).collect();
        sorted.sort();

        let mut out = String::new();
        for flag in sorted {
            out.push_str(flag);
            out.push('\n');
        }
        out
    }

    /// Parses a `.flags` sidecar payload (one flag per line, blanks
    /// ignored).
    pub fn from_sidecar(contents: &str) -> Self {
        Self(
            contents
                .lines()
                .filter(|line| !line.is_empty())
                .map(ToString::to_string)
                .collect(),
        )
    }
}

impl fmt::Display for Flags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut sorted: Vec<&str> = self.0.iter().map(String::as_str).collect();
        sorted.sort();
        write!(f, "{}", sorted.join(","))
    }
}

impl FromIterator<String> for Flags {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<'a> FromIterator<&'a str> for Flags {
    fn from_iter<I: IntoIterator<Item = &'a str>>(iter: I) -> Self {
        Self(iter.into_iter().map(ToString::to_string).collect())
    }
}

impl From<HashSet<String>> for Flags {
    fn from(set: HashSet<String>) -> Self {
        Self(set)
    }
}

impl From<Flags> for HashSet<String> {
    fn from(flags: Flags) -> Self {
        flags.0
    }
}

#[cfg(test)]
mod tests {
    use crate::flag::*;

    #[test]
    fn sidecar_round_trip() {
        let mut flags = Flags::default();
        flags.insert("$seen");
        flags.insert("$forwarded");
        flags.insert("custom");

        let serialized = flags.to_sidecar();
        let parsed = Flags::from_sidecar(&serialized);

        assert_eq!(parsed.len(), 3);
        assert!(parsed.contains("$seen"));
        assert!(parsed.contains("$forwarded"));
        assert!(parsed.contains("custom"));
    }

    #[test]
    fn sidecar_is_sorted() {
        let mut flags = Flags::default();
        flags.insert("zeta");
        flags.insert("alpha");
        flags.insert("middle");
        assert_eq!(flags.to_sidecar(), "alpha\nmiddle\nzeta\n");
    }

    #[test]
    fn from_sidecar_ignores_blanks() {
        let parsed = Flags::from_sidecar("$seen\n\n\n$forwarded\n");
        assert_eq!(parsed.len(), 2);
        assert!(parsed.contains("$seen"));
        assert!(parsed.contains("$forwarded"));
    }
}
