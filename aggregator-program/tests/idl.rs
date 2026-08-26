//! The committed IDL is what the guest source currently says.
//!
//! The artefact under `kanon-idl/` is a cache of a surface defined elsewhere, so
//! the only thing that keeps it true is regenerating it and comparing. A changed
//! instruction with a stale artefact would ship an SDK and a CLI built against a
//! surface the program no longer has, and nothing else in the build would notice.

use std::path::Path;
use std::process::Command;

#[test]
fn the_committed_idl_matches_the_guest_source() {
    let generated = Command::new(env!("CARGO_BIN_EXE_generate-idl"))
        .output()
        .expect("the generator builds and runs");
    assert!(
        generated.status.success(),
        "the generator failed: {}",
        String::from_utf8_lossy(&generated.stderr)
    );

    let committed_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("kanon-idl")
        .join("aggregator-idl.json");
    let committed = std::fs::read_to_string(&committed_path).expect("the artefact is committed");

    let generated = String::from_utf8(generated.stdout).expect("the IDL is UTF-8");

    assert_eq!(
        generated.trim(),
        committed.trim(),
        "`kanon-idl/aggregator-idl.json` is stale. Regenerate it:\n    \
         cargo run -p aggregator-program --bin generate-idl > kanon-idl/aggregator-idl.json"
    );
}
