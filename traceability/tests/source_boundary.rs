//! The `# harness-only` sources in `deny.toml` stay out of what ships.
//!
//! `deny.toml` is one file for every workspace, because two would drift on the
//! licence rules and those are the part that must stay identical
//! (`[M2-19:01]`). cargo-deny has no per-workspace source policy, so the seven
//! sources the end-to-end harness needs -- the Bedrock node behind the
//! sequencer's RPC client -- are permitted repository-wide by that one list.
//!
//! Without a check, "but the product does not use them" is a claim about today
//! that nothing enforces: a product dependency reaching one of those repos
//! tomorrow would pass the gate silently. @frenzox raised exactly that on #58.
//! These two tests are the enforcement, and they run in `build and test` on
//! every push rather than only where a chain exists.
//!
//! Both directions matter. The first is the guarantee. The second keeps the
//! marker honest: a `# harness-only` comment on a source the harness does not
//! actually resolve would be an exemption nobody is using, and the next
//! dependency to reach that repo -- from anywhere -- would inherit it.
//!
//! # What counts as shipped
//!
//! Every committed `Cargo.lock` except the harness's own. Deliberately wider
//! than the root: `methods/guest/Cargo.lock` resolves separately and is what the
//! guest ELF is built from, so a source reaching only the guest would ship while
//! a root-only check called it absent. @frenzox caught that too, on the first
//! version of this file.
//!
//! `m0/`'s root and its five guest lockfiles are in scope as well, which is
//! stricter than the exclusion `scripts/lez-sequencer.sh` uses. That script
//! skips `m0/` because `m0/` legitimately pins a *different LEZ revision*, which
//! is a reason about revisions and does not transfer: nothing in the measurement
//! workspace has any business resolving the sequencer's node either, and one
//! rule with no exception list is less to keep true. All seven of those
//! lockfiles are clean today, so the stricter rule costs nothing to adopt.
//!
//! Here rather than in `e2e/`, because `e2e`'s own tests need a running
//! sequencer and so run in one job only. This is the crate whose tests are
//! repository-wide gates; `tests/matrix.rs` is its sibling.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use traceability::repo_root;

/// The marker, spelled once.
const MARKER: &str = "# harness-only";

/// Every source in `deny.toml`'s `allow-git` carrying [`MARKER`].
///
/// Read as text rather than through a TOML parser for one reason: the marker is
/// a comment, and a parser discards comments. Keeping it a comment is
/// deliberate -- it sits on the line it describes, where someone editing the
/// list will see it, rather than in a second list that can disagree with the
/// first.
fn harness_only_sources(deny: &str) -> BTreeSet<String> {
    deny.lines()
        .filter(|line| line.contains(MARKER))
        .filter_map(|line| {
            let start = line.find('"')? + 1;
            let end = line[start..].find('"')? + start;
            Some(line[start..end].to_owned())
        })
        .collect()
}

/// Every distinct git source a lockfile resolves, as a bare URL.
///
/// A lockfile records `git+<url>?tag=v0.2.0#<sha>`, so the query and fragment
/// come off before the comparison: `deny.toml` names the repository and cargo
/// names the revision it picked out of it.
fn git_sources(lock: &str) -> BTreeSet<String> {
    lock.lines()
        .filter_map(|line| line.trim().strip_prefix("source = \"git+"))
        .filter_map(|rest| rest.strip_suffix('"'))
        .map(|url| {
            let end = url.find(['?', '#']).unwrap_or(url.len());
            url[..end].to_owned()
        })
        .collect()
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
}

/// Every committed lockfile that describes something delivered.
///
/// All of them but `e2e/Cargo.lock`, which is the harness's own and the one
/// place these sources belong. `target/` is skipped: it holds lockfiles of
/// dependencies cargo happened to check out, which are not this repository's to
/// police.
fn product_lockfiles() -> Vec<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if name == "target" || name == ".git" || name == "e2e" {
                continue;
            }
            if path.is_dir() {
                walk(&path, found);
            } else if name == "Cargo.lock" {
                found.push(path);
            }
        }
    }

    let root = repo_root();
    let mut found = Vec::new();
    walk(&root, &mut found);
    found.sort();

    // A walk that silently found nothing would pass every assertion below. These
    // two are the ones whose absence means the scan broke rather than that the
    // repository changed: the root workspace, and the guest -- whose omission was
    // the gap this file was widened to close, so it is the one to notice going
    // missing again.
    for expected in ["Cargo.lock", "methods/guest/Cargo.lock"] {
        assert!(
            found.contains(&root.join(expected)),
            "the lockfile scan did not find {expected}, so it is not scanning what \
             it claims to. Found: {found:?}"
        );
    }
    found
}

fn marked() -> BTreeSet<String> {
    let sources = harness_only_sources(&read(&repo_root().join("deny.toml")));
    assert!(
        !sources.is_empty(),
        "no `{MARKER}` source in deny.toml, so this gate is asserting nothing. \
         Either the marker was renamed -- update MARKER here with it -- or the \
         harness no longer needs its own sources, in which case delete them from \
         `allow-git` rather than leaving the allowance in place unenforced"
    );
    sources
}

#[test]
fn no_harness_only_source_is_in_any_product_lockfile() {
    let marked = marked();
    let root = repo_root();

    let leaked: Vec<String> = product_lockfiles()
        .iter()
        .flat_map(|lock| {
            let resolved = git_sources(&read(lock));
            let relative = lock
                .strip_prefix(&root)
                .unwrap_or(lock)
                .display()
                .to_string();
            marked
                .iter()
                .filter(|source| resolved.contains(*source))
                .map(move |source| format!("{relative}: {source}"))
                .collect::<Vec<_>>()
        })
        .collect();

    assert!(
        leaked.is_empty(),
        "a lockfile for something delivered now resolves a source marked \
         `{MARKER}`:\n  {}\n\n\
         `deny.toml` allows these for the end-to-end harness in `e2e/`, and they \
         are not vetted for anything that ships. Note which lockfile: the guest's \
         is resolved separately from the root's and is what the ELF is built from, \
         so a source reaching only the guest still ships. If a product crate \
         genuinely needs one, take the `{MARKER}` marker off that line and say in \
         `deny.toml` why the source is acceptable in a deliverable -- do not \
         silence this test.",
        leaked.join("\n  ")
    );
}

#[test]
fn every_harness_only_source_is_one_the_harness_actually_resolves() {
    let harness = git_sources(&read(&repo_root().join("e2e/Cargo.lock")));
    let marked = marked();

    let unused: Vec<&String> = marked.iter().filter(|s| !harness.contains(*s)).collect();

    assert!(
        unused.is_empty(),
        "these sources are marked `{MARKER}` but `e2e/Cargo.lock` does not \
         resolve them:\n  {}\n\n\
         So the allowance is not being used by the harness, and the next \
         dependency to reach one of those repositories -- from any workspace, \
         including one that ships -- would inherit it. Remove them from \
         `allow-git`.",
        unused
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
