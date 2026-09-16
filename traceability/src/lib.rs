//! The RFP-020 traceability matrix: model, loader and renderer.
//!
//! Three files make up the matrix and each has one job.
//! `requirements.toml` is the inventory, transcribed from RFP-020.
//! `matrix.toml` maps each requirement to the tasks that implement it and the
//! evidence that verifies it. `TRACEABILITY.md`, at the repository root, is
//! rendered from both and is what a reader opens.
//!
//! Nothing here decides whether the matrix is *right*: that is
//! `tests/matrix.rs`, which resolves every piece of evidence against the
//! repository and fails when one does not exist. Keeping the two apart means
//! the renderer cannot quietly paper over a missing test by omitting it.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

/// The requirement inventory, in RFP order.
#[derive(Debug, Deserialize)]
pub struct Requirements {
    #[serde(rename = "requirement")]
    pub requirements: Vec<Requirement>,
}

/// One numbered RFP-020 requirement.
#[derive(Debug, Deserialize)]
pub struct Requirement {
    pub id: String,
    pub category: String,
    pub kind: Kind,
    pub title: String,
}

/// Hard requirements must ship; soft ones are the RFP's stretch pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Hard,
    Soft,
}

/// The mapping, one row per requirement.
#[derive(Debug, Deserialize)]
pub struct Matrix {
    #[serde(rename = "row")]
    pub rows: Vec<Row>,
}

/// What implements a requirement, and what stands behind the claim.
#[derive(Debug, Deserialize)]
pub struct Row {
    pub id: String,
    pub status: Status,
    /// Task ids from the delivery plan, e.g. `M1-13`.
    pub tasks: Vec<String>,
    /// Paths, relative to the repository root, that must exist.
    pub implemented_in: Vec<String>,
    /// Evidence in `test:`, `ci:` or `doc:` form. See [`Evidence`].
    #[serde(default)]
    pub verified_by: Vec<String>,
    pub note: String,
}

/// How far along a requirement is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Planned,
    Partial,
    Verified,
}

impl Status {
    /// The word used in `TRACEABILITY.md`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Partial => "partial",
            Self::Verified => "verified",
        }
    }
}

/// A resolvable claim that something verifies a requirement.
///
/// The point of the three forms is that each can be checked mechanically. A
/// prose claim could not be, which is how traceability matrices normally rot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    /// A test function that exists somewhere in this repository.
    Test(String),
    /// A job id in `.github/workflows/`.
    Ci(String),
    /// A delivered document, at a path that exists.
    Doc(String),
}

impl Evidence {
    /// Parses one `verified_by` entry.
    ///
    /// # Errors
    ///
    /// Returns the offending string when it carries no recognised prefix, since
    /// an unrecognised form would otherwise be silently unchecked.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.split_once(':') {
            Some(("test", name)) => Ok(Self::Test(name.to_owned())),
            Some(("ci", job)) => Ok(Self::Ci(job.to_owned())),
            Some(("doc", path)) => Ok(Self::Doc(path.to_owned())),
            _ => Err(format!(
                "`{raw}` is not evidence: expected `test:<fn>`, `ci:<job>` or `doc:<path>`"
            )),
        }
    }

    /// How the entry is rendered in `TRACEABILITY.md`.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Test(name) => format!("`{name}`"),
            Self::Ci(job) => format!("CI job `{job}`"),
            Self::Doc(path) => format!("`{path}`"),
        }
    }
}

/// The repository root, derived from this crate's location.
#[must_use]
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the traceability crate always sits one level below the repository root")
        .to_path_buf()
}

/// Loads and parses `requirements.toml` and `matrix.toml`.
///
/// # Panics
///
/// On unreadable or malformed input. Both files are committed next to this
/// code, so either failure is a broken checkout rather than a runtime
/// condition worth threading a result through.
#[must_use]
pub fn load() -> (Requirements, Matrix) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |name: &str| {
        let path = dir.join(name);
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("reading {}: {err}", path.display()))
    };

    let requirements: Requirements =
        toml::from_str(&read("requirements.toml")).unwrap_or_else(|err| {
            panic!("parsing requirements.toml: {err}");
        });
    let matrix: Matrix = toml::from_str(&read("matrix.toml")).unwrap_or_else(|err| {
        panic!("parsing matrix.toml: {err}");
    });

    (requirements, matrix)
}

/// Where the rendered matrix belongs.
#[must_use]
pub fn rendered_path() -> PathBuf {
    repo_root().join("TRACEABILITY.md")
}

fn escape(cell: &str) -> String {
    cell.replace('|', r"\|")
}

/// Replaces hyphens with U+2011 NON-BREAKING HYPHEN.
///
/// GitHub wraps lines at hyphens, including inside code spans, which split
/// `M1-10` into "M1" and "-10" on separate lines and `verifier-core` into
/// "verifier-" and "core". U+2011 renders identically to an ordinary hyphen and
/// cannot be broken.
///
/// Used only in the table, where columns are narrow enough for it to happen.
/// The prose below keeps ordinary hyphens, so the requirement text and the
/// notes stay searchable and copy-pastable -- and every task id appears there
/// too, which is what keeps the document greppable for `M1-10` despite this.
fn unbreakable(text: &str) -> String {
    text.replace('-', "\u{2011}")
}

fn join_code(items: &[String]) -> String {
    if items.is_empty() {
        return "—".to_owned();
    }
    items
        .iter()
        .map(|item| format!("`{}`", unbreakable(item)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Renders `TRACEABILITY.md` from the two source files.
///
/// Deterministic: category order follows first appearance in
/// `requirements.toml` and rows follow the inventory, so a diff shows a change
/// in the matrix rather than a change in iteration order.
///
/// # Panics
///
/// If a requirement has no row. `tests/matrix.rs` reports that case properly;
/// reaching it here means the renderer was run against a matrix that the test
/// suite would already have rejected.
#[must_use]
pub fn render(requirements: &Requirements, matrix: &Matrix) -> String {
    let rows: BTreeMap<&str, &Row> = matrix
        .rows
        .iter()
        .map(|row| (row.id.as_str(), row))
        .collect();

    let mut categories: Vec<&str> = Vec::new();
    for requirement in &requirements.requirements {
        if !categories.contains(&requirement.category.as_str()) {
            categories.push(&requirement.category);
        }
    }

    let count = |status: Status| {
        matrix
            .rows
            .iter()
            .filter(|row| row.status == status)
            .count()
    };
    let total = matrix.rows.len();

    let mut out = String::new();
    out.push_str(
        "# Requirement traceability\n\
         \n\
         Every RFP-020 requirement, what implements it, and what verifies it.\n\
         \n\
         **Generated. Do not edit.** The sources are `traceability/requirements.toml`\n\
         (the inventory, transcribed from RFP-020) and `traceability/matrix.toml`\n\
         (the mapping). Regenerate with:\n\
         \n\
         ```sh\n\
         cargo run -p traceability --bin render-traceability\n\
         ```\n\
         \n\
         `cargo test -p traceability` fails if this file and those sources disagree,\n\
         if any evidence below names a test, CI job or document that does not exist,\n\
         or if a task id written anywhere in the repository appears in no row. That\n\
         check is the deliverable, not the table: an unverified matrix drifts into\n\
         fiction by the third milestone.\n\
         \n\
         Statuses are `planned` (nothing implemented yet), `partial` (partly\n\
         satisfied, or satisfied by M0 evidence that later tasks extend) and\n\
         `verified` (satisfied now, with a test or a CI gate behind it, never a\n\
         document alone). M5-06 is the task that has to leave every row `verified`.\n\
         \n\
         Not tracked here: the Servicing obligations from the proposal's *Servicing\n\
         and SLA* section. They are contractual rather than code and have no test to\n\
         name; monthly operating reports are their evidence.\n\
         \n",
    );

    let _ = writeln!(
        out,
        "Right now: **{} verified, {} partial, {} planned**, of {total} requirements.\n",
        count(Status::Verified),
        count(Status::Partial),
        count(Status::Planned),
    );

    for category in categories {
        let _ = writeln!(out, "## {category}\n");
        out.push_str("| Requirement | Status | Tasks | Implemented in | Verified by |\n");
        out.push_str("| --- | --- | --- | --- | --- |\n");

        let in_category: Vec<&Requirement> = requirements
            .requirements
            .iter()
            .filter(|requirement| requirement.category == category)
            .collect();

        for requirement in &in_category {
            let row = rows
                .get(requirement.id.as_str())
                .unwrap_or_else(|| panic!("{} has no row in matrix.toml", requirement.id));
            let evidence: Vec<String> = row
                .verified_by
                .iter()
                .map(|raw| {
                    Evidence::parse(raw)
                        .unwrap_or_else(|err| panic!("{}: {err}", requirement.id))
                        .render()
                })
                .collect();
            let evidence = if evidence.is_empty() {
                "—".to_owned()
            } else {
                evidence.join(", ")
            };
            let soft = if requirement.kind == Kind::Soft {
                " *(soft)*"
            } else {
                ""
            };

            // The id only. Requirement titles run to 250 characters, and a
            // markdown table sizes its columns to fit their content: one long
            // cell squeezes every other column until short tokens like `M1-10`
            // wrap mid-word. The titles are directly below instead, where prose
            // belongs and where they are actually readable.
            let _ = writeln!(
                out,
                "| **{}**{soft} | {} | {} | {} | {} |",
                requirement.id,
                row.status.label(),
                join_code(&row.tasks),
                join_code(&row.implemented_in),
                unbreakable(&escape(&evidence)),
            );
        }
        out.push('\n');

        for requirement in in_category {
            let row = &rows[requirement.id.as_str()];
            // No pipe-escaping here: that is a table concern, and this is prose.
            let _ = writeln!(out, "**{}** — {}", requirement.id, requirement.title);
            if row.note.is_empty() {
                out.push('\n');
            } else {
                let _ = writeln!(out, "\n{}\n", row.note);
            }
        }
    }

    // Exactly one trailing newline. Each note above is written with a blank line
    // after it, which reads correctly between sections and leaves a stray blank
    // line at EOF for the last one -- enough for `git diff --check` to flag the
    // generated file as a whitespace error on every regeneration.
    while out.ends_with("\n\n") {
        out.pop();
    }

    out
}
