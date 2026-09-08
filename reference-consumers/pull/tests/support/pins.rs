//! A guest manifest's `[patch.crates-io]` entries, read one way.
//!
//! Two gates compare pin lines across guest workspaces: `guest_pins.rs` here,
//! which locks the two verifying guests to each other, and
//! `accelerator_pins.rs` in `reference-consumer-aggregator-read`, which asserts
//! the reading guest's narrower list. Each grew its own reader and the two
//! disagreed about comments, so one gate could normalise a manifest line the
//! other would not and a pin could move under exactly one of them.
//!
//! Included by path rather than shared through a crate, the way
//! `verifier-core/tests/support/vectors.rs` is: each file under `tests/` is its
//! own crate, so a reader two of them need is otherwise written twice.

#![allow(dead_code)] // Each gate reads a different subset.

use std::path::Path;

/// A manifest, named relative to the repository root.
///
/// Root-relative rather than crate-relative because both gates name manifests
/// outside their own crate, and a path that means something different depending
/// on which file included it is the shape of problem this module exists to
/// remove. Both including crates sit at `reference-consumers/<name>`, so the
/// root is two parents up from either one.
pub fn manifest(relative: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the repository root");
    let path = root.join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} is readable: {err}", path.display()))
}

/// The `[patch.crates-io]` entries of a manifest, as normalised lines.
///
/// Text rather than a TOML parse: the section is a few lines of a fixed shape,
/// and a parser would be a dependency for both guest workspaces to no end.
/// Whole lines rather than crate names, so a changed tag moves a line: the set
/// alone would let a pin drift to another revision while still reading as
/// `["sha2"]`.
pub fn pins(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .skip_while(|line| line.trim() != "[patch.crates-io]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .map(without_comment)
        .filter(|line| !line.is_empty())
        .collect()
}

/// The one entry naming `crate_name`, or `None`.
pub fn pin_for(manifest: &str, crate_name: &str) -> Option<String> {
    pins(manifest)
        .into_iter()
        .find(|line| line.starts_with(crate_name))
}

/// Everything before the first `#` outside a TOML string, trimmed.
///
/// A whole-line comment comes back empty and is dropped by the caller. Strings
/// are skipped rather than scanned because a pin's value can carry a `#` of its
/// own, and cutting there would shorten the line one gate compares while
/// leaving the other's intact.
///
/// Both of TOML's single-line string forms, because they escape differently and
/// getting one wrong is the same defect twice: a basic string honours
/// backslashes, so `"a\"# b"` is one string and not a comment, while a literal
/// string has no escapes at all and `'release#1'` ends only at its closing
/// quote. Multi-line strings are not handled and cannot be: this whole reader
/// is line-based, and a pin has never been written across lines.
fn without_comment(line: &str) -> String {
    let mut chars = line.char_indices();
    while let Some((at, ch)) = chars.next() {
        match ch {
            '#' => return line[..at].trim().to_owned(),
            // Basic string: a backslash consumes whatever follows it, so an
            // escaped quote does not end the string.
            '"' => {
                while let Some((_, inner)) = chars.next() {
                    match inner {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            // Literal string: no escapes, so it ends at the next quote.
            '\'' => {
                for (_, inner) in chars.by_ref() {
                    if inner == '\'' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    line.trim().to_owned()
}
