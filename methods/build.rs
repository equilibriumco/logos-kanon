use std::{env, fs, path::Path, path::PathBuf};

use risc0_build::{DockerOptionsBuilder, GuestOptionsBuilder};

/// The guest builder image, pinned by digest.
///
/// `risc0-build`'s own default is `r0.1.88.0`, and taking it would swap the guest
/// compiler under every figure in `COSTS.md`, which were all measured on guest rustc
/// 1.97.0. `r0.1.97.0` carries that compiler, so containerising the build changes where
/// a guest is compiled without changing what compiles it. ADR 8's pins and this one say
/// the same thing and have to move together.
///
/// The digest is the load-bearing half. A tag is a mutable pointer, so two builds of
/// this commit far enough apart could resolve one to different image contents and
/// produce different program ids -- the failure this file exists to prevent, arriving
/// slowly instead of immediately. `repo:tag@sha256:...` is a valid reference and Docker
/// resolves the digest, so the tag survives for legibility and decides nothing.
const GUEST_BUILDER_IMAGE: &str =
    "r0.1.97.0@sha256:7ae0a27fa72b10fb5c1066005bd949ae43dc4ead8a284f2d6572b99c82ab4468";

/// `risc0-build` consults this *before* the image configured below, so a pin that
/// ignored it would be a pin in name only.
const TAG_OVERRIDE: &str = "RISC0_DOCKER_CONTAINER_TAG";

/// The escape hatch, and what it costs to use.
///
/// The builder image is x86_64 linux, so a developer on another architecture would
/// otherwise have no way to build at all; `KANON_SEQUENCER_RUNTIME=host` is the same
/// escape for the same reason (ADR 34). What a host build must not be used for is
/// anything published, because its image ids depend on the checkout path, which is the
/// whole point of the containerised route.
///
/// Cycle figures are not affected either way. They are a property of the instructions,
/// and the difference between the two routes is confined to symbol names, so `cost.rs`
/// and `read_cost.rs` pass identically under both.
const ESCAPE_HATCH: &str = "KANON_GUEST_BUILD";

fn main() {
    println!("cargo:rerun-if-env-changed={ESCAPE_HATCH}");
    println!("cargo:rerun-if-env-changed={TAG_OVERRIDE}");
    if env::var(ESCAPE_HATCH).as_deref() == Ok("host") {
        println!(
            "cargo:warning={ESCAPE_HATCH}=host: guest image ids will depend on this \
             checkout's path and must not be published"
        );
        risc0_build::embed_methods();
        return;
    }

    if let Ok(tag) = env::var(TAG_OVERRIDE) {
        assert_eq!(
            tag, GUEST_BUILDER_IMAGE,
            "{TAG_OVERRIDE} is set to `{tag}`, which `risc0-build` would use ahead of \
             the digest this crate pins. A guest built from another image is a different \
             program: its ids are not the ones the ADR records. Unset it, or use \
             {ESCAPE_HATCH}=host if you meant to build outside the container."
        );
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets this"));
    let root = manifest_dir
        .parent()
        .expect("methods/ sits under the workspace root")
        .to_path_buf();

    let docker = DockerOptionsBuilder::default()
        // The whole repository, because a guest workspace depends on crates above it --
        // `verifier-core` and `kanon-idl` among them -- and the container sees only what
        // is copied into it.
        .root_dir(root)
        .docker_container_tag(GUEST_BUILDER_IMAGE)
        .build()
        .expect("every field the builder requires is set");

    let options = GuestOptionsBuilder::default()
        .use_docker(docker)
        .build()
        .expect("every field the builder requires is set");

    let guests = guest_packages(&manifest_dir);
    risc0_build::embed_methods_with_options(
        guests
            .iter()
            .map(|guest| (guest.as_str(), options.clone()))
            .collect(),
    );
}

/// The guest packages, read from the metadata `risc0-build` reads to find them.
///
/// Derived rather than listed, because a list here would be a second copy of one that
/// already exists, and the two failing to agree is silent: `risc0-build` applies default
/// options to any guest absent from the map it is handed, and the default is a host
/// build. A fourth entry added to `[package.metadata.risc0]` and not here would compile
/// outside the container and go back to having a directory in its program id, with
/// nothing to say so.
fn guest_packages(manifest_dir: &Path) -> Vec<String> {
    let manifest: toml::Value = fs::read_to_string(manifest_dir.join("Cargo.toml"))
        .expect("this crate has a manifest")
        .parse()
        .expect("and it is TOML");

    manifest
        .get("package")
        .and_then(|package| package.get("metadata"))
        .and_then(|metadata| metadata.get("risc0"))
        .and_then(|risc0| risc0.get("methods"))
        .and_then(|methods| methods.as_array())
        .expect("`[package.metadata.risc0]` declares the guests, and is what finds them")
        .iter()
        .map(|path| {
            let path = path.as_str().expect("a guest path is a string");
            let guest: toml::Value = fs::read_to_string(manifest_dir.join(path).join("Cargo.toml"))
                .unwrap_or_else(|_| panic!("guest `{path}` has a manifest"))
                .parse()
                .unwrap_or_else(|_| panic!("guest `{path}`'s manifest is TOML"));
            guest
                .get("package")
                .and_then(|package| package.get("name"))
                .and_then(|name| name.as_str())
                .unwrap_or_else(|| panic!("guest `{path}` is a named package"))
                .to_owned()
        })
        .collect()
}
