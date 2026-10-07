//! Keeping the user's filesystem out of logs and error text.
//!
//! The app writes a rotating log beside its data directory, and a user
//! reporting a problem is asked to send that file. Every path written into it
//! therefore leaves the machine. A raw
//! `C:\Users\<account>\AppData\Roaming\app\library.json` discloses the account
//! name and the shape of the user's disk, to diagnose a failure that the file
//! name alone almost always describes just as well.
//!
//! `parser.rs` already reduces paths to a file name for its user-facing
//! messages. This module generalises that convention so the rest of the app can
//! follow it.
//!
//! Redaction happens where the text is *built* rather than where it is logged,
//! so there is no path a later caller can leak by remembering to log but
//! forgetting to redact.

use std::path::Path;

/// Shown in place of a path that has no final component to report, such as a
/// filesystem root.
const UNKNOWN: &str = "<path>";

/// Reduce a path to its final component.
///
/// `C:\Users\amy\AppData\Roaming\app\library.json` becomes `library.json`,
/// which tells a reader which file failed without telling them whose machine
/// it was.
///
/// Returns [`UNKNOWN`] when there is no final component, so a root or an empty
/// path degrades to a placeholder instead of to a partial path that would leak
/// the same information by another route.
pub fn path(value: &Path) -> String {
    value
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| UNKNOWN.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_profile_path_is_reduced_to_its_file_name() {
        let full = PathBuf::from(r"C:\Users\someone\AppData\Roaming\app\library.json");
        assert_eq!(path(&full), "library.json");
    }

    #[test]
    fn no_part_of_the_account_name_survives() {
        let full = PathBuf::from(r"C:\Users\someone\Documents\books\novel.epub");
        let redacted = path(&full);
        assert!(
            !redacted.contains("someone"),
            "account name leaked: {redacted}"
        );
        assert!(!redacted.contains("Documents"), "layout leaked: {redacted}");
        assert!(!redacted.contains('\\'), "separator leaked: {redacted}");
    }

    #[test]
    fn a_relative_path_is_reduced_the_same_way() {
        assert_eq!(path(&PathBuf::from("books/abc.epub")), "abc.epub");
    }

    #[test]
    fn a_root_degrades_to_a_placeholder_rather_than_a_partial_path() {
        assert_eq!(path(&PathBuf::from("/")), UNKNOWN);
    }
}
