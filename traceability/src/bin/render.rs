//! Regenerates `TRACEABILITY.md` from the matrix sources.
//!
//! ```sh
//! cargo run -p traceability --bin render-traceability
//! ```
//!
//! CI does not run this; it runs `cargo test -p traceability`, which fails when
//! the committed file and the sources disagree. Rendering in CI instead would
//! mean the readable matrix could change without anyone reviewing the change.

fn main() {
    let (requirements, matrix) = traceability::load();
    let rendered = traceability::render(&requirements, &matrix);
    let path = traceability::rendered_path();

    let unchanged = std::fs::read_to_string(&path).is_ok_and(|current| current == rendered);
    if unchanged {
        println!("{} is already up to date", path.display());
        return;
    }

    std::fs::write(&path, rendered)
        .unwrap_or_else(|err| panic!("writing {}: {err}", path.display()));
    println!("wrote {}", path.display());
}
