//! Generic source registry.
//!
//! Containers in oxideav take a `Box<dyn ReadSeek>`; this module is what
//! turns a URI into one. External drivers (e.g. `oxideav-http`)
//! register themselves into a [`SourceRegistry`] for additional
//! schemes. The built-in `file` driver is provided by the
//! `oxideav-source` re-export shim crate (so this core module stays
//! free of `std::fs` use cases that callers may want to swap out).

use std::collections::HashMap;

use super::container::ReadSeek;
use crate::{Error, Result};

/// Function signature for a source driver. Receives the full URI string
/// and returns an opened reader.
pub type OpenSourceFn = fn(uri: &str) -> Result<Box<dyn ReadSeek>>;

/// Registry mapping URI schemes to opener functions.
#[derive(Default)]
pub struct SourceRegistry {
    schemes: HashMap<String, OpenSourceFn>,
}

impl SourceRegistry {
    /// Empty registry. Callers must register at least the `file` driver
    /// before calling [`open`](Self::open).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an opener for a scheme. Schemes are normalised to ASCII
    /// lowercase. Replaces any prior registration.
    pub fn register(&mut self, scheme: &str, opener: OpenSourceFn) {
        self.schemes.insert(scheme.to_ascii_lowercase(), opener);
    }

    /// Open a URI. The URI's scheme determines which opener runs; bare
    /// paths (no scheme) and unrecognised schemes both fall back to the
    /// `file` driver if it is registered.
    pub fn open(&self, uri_str: &str) -> Result<Box<dyn ReadSeek>> {
        let (scheme, _) = split_scheme(uri_str);
        let scheme = scheme.to_ascii_lowercase();
        if let Some(opener) = self.schemes.get(&scheme) {
            return opener(uri_str);
        }
        // Fall back to file driver for unknown schemes.
        if let Some(opener) = self.schemes.get("file") {
            return opener(uri_str);
        }
        Err(Error::Unsupported(format!(
            "no source driver for scheme '{scheme}' (URI: {uri_str})"
        )))
    }

    /// Iterate the registered schemes (for diagnostics).
    pub fn schemes(&self) -> impl Iterator<Item = &str> {
        self.schemes.keys().map(|s| s.as_str())
    }
}

/// Split a URI into `(scheme, rest)`. Bare paths (no scheme) report scheme
/// `"file"` and `rest = uri`. Path-like inputs that happen to start with
/// `c:` on Windows are treated as bare paths.
pub(crate) fn split_scheme(uri: &str) -> (&str, &str) {
    if let Some(idx) = uri.find(':') {
        let (scheme, rest) = uri.split_at(idx);
        let rest = &rest[1..]; // skip ':'

        // Reject single-letter scheme that looks like a Windows drive letter.
        if scheme.len() == 1 && scheme.chars().next().unwrap().is_ascii_alphabetic() {
            return ("file", uri);
        }

        // Scheme must be ASCII alphanumeric / `+` / `-` / `.`, starting with a letter.
        let valid = !scheme.is_empty()
            && scheme.chars().next().unwrap().is_ascii_alphabetic()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));

        if !valid {
            return ("file", uri);
        }

        // Strip leading `//` from rest if present.
        let rest = rest.strip_prefix("//").unwrap_or(rest);
        return (scheme, rest);
    }
    ("file", uri)
}
