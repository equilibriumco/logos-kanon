//! The traceability gate (M1-06).
//!
//! Supportability 3 asks for at least one test per hard requirement. A matrix
//! claiming that is worth nothing unless something checks the claim, so these
//! tests resolve every entry against the repository: a named test function has
//! to exist, a named CI job has to exist in a workflow, a named document has to
//! be on disk, and a named implementation path has to be a real file or
//! directory. A row cannot be marked `verified` on the strength of a document
//! alone.
//!
//! Two failures are expected during the milestones and are not defects: a row
//! whose evidence is renamed (rename the test, update the row) and a
//! `TRACEABILITY.md` that no longer matches its sources (rerun the renderer).
//! M5-06 is the task that requires every row to read `verified`.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use traceability::{Evidence, Kind, Matrix, Requirements, Row, Status};

/// RFP-020's own taxonomy, counted.
///
/// Transcribing an inventory is exactly the kind of work that silently loses an
/// entry, so the counts are asserted rather than trusted. `OS1` is the *Open
/// Source Requirement* section, which is a hard obligation stated outside the
/// numbered lists.
const EXPECTED_COUNTS: &[(&str, usize)] = &[
    ("Functionality", 9),
    ("Usability", 7),
    ("Reliability", 4),
    ("Performance", 3),
    ("Supportability", 7),
    ("Adaptor Security", 3),
    ("Open source", 1),
    ("Soft", 2),
];

/// Every file under `root` carrying one of `extensions`.
///
/// `target` is build output and `.git` is history: neither holds anything this
/// repository would want to make a claim about.
fn sources(root: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    fn walk(dir: &Path, extensions: &[&str], found: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if name == "target" || name == ".git" {
                continue;
            }
            if path.is_dir() {
                walk(&path, extensions, found);
            } else if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| extensions.contains(&ext))
            {
                found.push(path);
            }
        }
    }

    let mut found = Vec::new();
    walk(root, extensions, &mut found);
    found
}

fn all_rust_source_text(root: &Path) -> String {
    sources(root, &["rs"])
        .iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `M<milestone>-<nn>` task id written in `text`, with an optional
/// trailing letter for a task inserted between two existing ones.
///
/// Hand-rolled rather than a regex, for the same reason `ci_jobs` reads YAML
/// positionally: the pattern is fixed and a dependency would cost more than the
/// scan is worth. `M0-` ids are out of range deliberately — M0 is delivered and
/// its tasks answer to the report rather than to this matrix.
fn scan_task_ids(text: &str, into: &mut BTreeSet<String>) {
    let bytes = text.as_bytes();
    for (start, _) in text.match_indices('M') {
        // A task id is a whole word: `RFP-M1-04` or `xM1-04` are not one.
        if start > 0 {
            let before = bytes[start - 1];
            if before.is_ascii_alphanumeric() || before == b'-' || before == b'_' {
                continue;
            }
        }
        let rest = &bytes[start..];
        if rest.len() < 5
            || !(b'1'..=b'5').contains(&rest[1])
            || rest[2] != b'-'
            || !rest[3].is_ascii_digit()
            || !rest[4].is_ascii_digit()
        {
            continue;
        }
        let end = if rest.get(5).is_some_and(u8::is_ascii_lowercase) {
            6
        } else {
            5
        };
        // `M1-045` is not `M1-04`, and `M1-04ab` is not `M1-04a`.
        if rest
            .get(end)
            .is_some_and(|next| next.is_ascii_alphanumeric() || *next == b'_')
        {
            continue;
        }
        into.insert(String::from_utf8_lossy(&rest[..end]).into_owned());
    }
}

/// Job ids declared in `.github/workflows/`.
///
/// Read positionally rather than with a YAML parser: a job id is a key indented
/// two spaces under `jobs:`, and adding a YAML dependency to check that would be
/// more machinery than the check is worth.
fn ci_jobs(root: &Path) -> BTreeSet<String> {
    let mut jobs = BTreeSet::new();
    let Ok(entries) = fs::read_dir(root.join(".github/workflows")) else {
        return jobs;
    };

    for entry in entries.flatten() {
        let Ok(text) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let mut in_jobs = false;
        for line in text.lines() {
            if line.starts_with("jobs:") {
                in_jobs = true;
                continue;
            }
            if !in_jobs {
                continue;
            }
            // A non-indented, non-empty line ends the `jobs:` block.
            if !line.starts_with(' ') && !line.trim().is_empty() {
                in_jobs = false;
                continue;
            }
            let Some(rest) = line.strip_prefix("  ") else {
                continue;
            };
            if rest.starts_with(' ') || rest.starts_with('#') {
                continue;
            }
            if let Some(id) = rest.strip_suffix(':') {
                jobs.insert(id.to_owned());
            }
        }
    }
    jobs
}

fn rows_by_id(matrix: &Matrix) -> BTreeMap<&str, &Row> {
    matrix
        .rows
        .iter()
        .map(|row| (row.id.as_str(), row))
        .collect()
}

fn loaded() -> (Requirements, Matrix) {
    traceability::load()
}

#[test]
fn requirement_ids_are_unique() {
    let (requirements, matrix) = loaded();

    let mut seen = BTreeSet::new();
    for requirement in &requirements.requirements {
        assert!(
            seen.insert(requirement.id.clone()),
            "{} appears twice in requirements.toml",
            requirement.id
        );
    }

    let mut seen = BTreeSet::new();
    for row in &matrix.rows {
        assert!(
            seen.insert(row.id.clone()),
            "{} appears twice in matrix.toml",
            row.id
        );
    }
}

#[test]
fn every_requirement_has_exactly_one_row() {
    let (requirements, matrix) = loaded();
    let rows = rows_by_id(&matrix);

    let missing: Vec<&str> = requirements
        .requirements
        .iter()
        .map(|requirement| requirement.id.as_str())
        .filter(|id| !rows.contains_key(id))
        .collect();
    assert!(missing.is_empty(), "no row in matrix.toml for: {missing:?}");

    let known: BTreeSet<&str> = requirements
        .requirements
        .iter()
        .map(|requirement| requirement.id.as_str())
        .collect();
    let orphans: Vec<&str> = matrix
        .rows
        .iter()
        .map(|row| row.id.as_str())
        .filter(|id| !known.contains(id))
        .collect();
    assert!(
        orphans.is_empty(),
        "matrix.toml has rows for requirements that do not exist: {orphans:?}"
    );
}

#[test]
fn the_inventory_matches_the_rfp_taxonomy() {
    let (requirements, _) = loaded();

    for &(category, expected) in EXPECTED_COUNTS {
        let actual = requirements
            .requirements
            .iter()
            .filter(|requirement| requirement.category == category)
            .count();
        assert_eq!(
            actual, expected,
            "{category} should have {expected} requirements per RFP-020, found {actual}. \
             If the RFP itself changed, change this count in the same commit."
        );
    }

    let expected_total: usize = EXPECTED_COUNTS.iter().map(|&(_, count)| count).sum();
    assert_eq!(
        requirements.requirements.len(),
        expected_total,
        "requirements.toml carries a category that EXPECTED_COUNTS does not know about"
    );
}

#[test]
fn task_ids_are_well_formed() {
    let (_, matrix) = loaded();

    for row in &matrix.rows {
        for task in &row.tasks {
            let (milestone, rest) = task
                .split_once('-')
                .unwrap_or_else(|| panic!("{}: task `{task}` is not `M<n>-<nn>`", row.id));
            assert!(
                milestone.len() == 2
                    && milestone.starts_with('M')
                    && milestone[1..].chars().all(|c| c.is_ascii_digit()),
                "{}: task `{task}` has no `M<n>` milestone",
                row.id
            );
            // A trailing letter is how a task is inserted between two existing
            // ids without renumbering everything after it, e.g. `M1-04a`.
            let digits = rest.trim_end_matches(|c: char| c.is_ascii_lowercase());
            assert!(
                !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()),
                "{}: task `{task}` has no number",
                row.id
            );
        }
    }
}

#[test]
fn every_task_written_anywhere_is_owned_by_a_row() {
    let (_, matrix) = loaded();
    let root = traceability::repo_root();

    let owned: BTreeSet<&str> = matrix
        .rows
        .iter()
        .flat_map(|row| row.tasks.iter().map(String::as_str))
        .collect();

    let mut mentioned: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in sources(&root, &["rs", "toml", "md", "yml", "yaml", "sh", "nix"]) {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let mut found = BTreeSet::new();
        scan_task_ids(&text, &mut found);
        let shown = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        for id in found {
            mentioned.entry(id).or_default().insert(shown.clone());
        }
    }

    let orphans: Vec<String> = mentioned
        .iter()
        .filter(|(id, _)| !owned.contains(id.as_str()))
        .map(|(id, files)| {
            format!(
                "{id} (written in {})",
                files.iter().cloned().collect::<Vec<_>>().join(", ")
            )
        })
        .collect();

    assert!(
        orphans.is_empty(),
        "these task ids are written in the repository but appear in no row's `tasks`, \
         so no requirement claims them:\n  {}\n\
         Every task exists to satisfy something. Add it to the row it serves, or stop \
         naming it.",
        orphans.join("\n  ")
    );
}

#[test]
fn implementation_paths_exist() {
    let (_, matrix) = loaded();
    let root = traceability::repo_root();

    for row in &matrix.rows {
        for path in &row.implemented_in {
            assert!(
                root.join(path).exists(),
                "{}: `implemented_in` names {path}, which does not exist",
                row.id
            );
        }
    }
}

#[test]
fn every_piece_of_evidence_resolves() {
    let (_, matrix) = loaded();
    let root = traceability::repo_root();
    let sources = all_rust_source_text(&root);
    let jobs = ci_jobs(&root);

    for row in &matrix.rows {
        for raw in &row.verified_by {
            match Evidence::parse(raw).unwrap_or_else(|err| panic!("{}: {err}", row.id)) {
                Evidence::Test(name) => assert!(
                    sources.contains(&format!("fn {name}(")),
                    "{}: no test function `{name}` in the repository",
                    row.id
                ),
                Evidence::Ci(job) => assert!(
                    jobs.contains(&job),
                    "{}: no CI job `{job}` in .github/workflows (found: {jobs:?})",
                    row.id
                ),
                Evidence::Doc(path) => assert!(
                    root.join(&path).exists(),
                    "{}: evidence document {path} does not exist",
                    row.id
                ),
            }
        }
    }
}

#[test]
fn claimed_progress_is_backed_by_evidence() {
    let (_, matrix) = loaded();

    for row in &matrix.rows {
        match row.status {
            Status::Planned => assert!(
                row.verified_by.is_empty(),
                "{}: a planned requirement should not name evidence; \
                 if the evidence is real the status is at least partial",
                row.id
            ),
            Status::Partial => assert!(
                !row.verified_by.is_empty(),
                "{}: partial needs at least one piece of evidence",
                row.id
            ),
            Status::Verified => {
                let executable = row.verified_by.iter().any(|raw| {
                    matches!(
                        Evidence::parse(raw),
                        Ok(Evidence::Test(_) | Evidence::Ci(_))
                    )
                });
                assert!(
                    executable,
                    "{}: verified requires a test or a CI job, not documents alone",
                    row.id
                );
            }
        }
    }
}

#[test]
fn hard_requirements_name_the_task_that_delivers_them() {
    let (requirements, matrix) = loaded();
    let rows = rows_by_id(&matrix);

    for requirement in &requirements.requirements {
        if requirement.kind != Kind::Hard {
            // The two soft requirements are stretch scope, gated on the M0 cost
            // result and on RFP-019 existing, so they legitimately have no task.
            continue;
        }
        let row = &rows[requirement.id.as_str()];
        assert!(
            !row.tasks.is_empty(),
            "{}: a hard requirement with no task is unowned scope",
            requirement.id
        );
    }
}

#[test]
fn rendered_matrix_is_current() {
    let (requirements, matrix) = loaded();
    let path = traceability::rendered_path();
    let committed =
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("reading {}: {err}", path.display()));

    assert_eq!(
        committed,
        traceability::render(&requirements, &matrix),
        "TRACEABILITY.md is out of date. \
         Run: cargo run -p traceability --bin render-traceability"
    );
}
