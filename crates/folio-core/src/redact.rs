//! Keeps secrets out of `Debug` output.

use std::fmt;

/// Formats an optional secret as `Some(<redacted>)` or `None`, so that logging
/// a config with `{:?}` shows whether a secret is set but never its value.
pub struct Redacted<'a>(pub &'a Option<String>);

impl fmt::Debug for Redacted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(_) => f.write_str("Some(<redacted>)"),
            None => f.write_str("None"),
        }
    }
}
