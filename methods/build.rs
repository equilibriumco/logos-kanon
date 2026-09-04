use std::{collections::BTreeSet, env, fs, path::Path, path::PathBuf};

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

/// How the guests in this build were compiled, handed to the crate so something other
/// than a log line can tell.
///
/// `cargo:warning` is what the escape hatch had to announce itself with, and a warning is
/// lost in a CI log and easier to lose locally -- while what it announces is that the
/// program ids in this build have a directory in them again. `the_guests_were_built_in_a_container`
/// asserts this instead, which costs nothing when the guests legitimately change and
/// fails only in the one case that matters.
const BUILD_MODE: &str = "KANON_GUEST_BUILD_MODE";

fn main() {
    println!("cargo:rerun-if-env-changed={ESCAPE_HATCH}");
    println!("cargo:rerun-if-env-changed={TAG_OVERRIDE}");
    if env::var(ESCAPE_HATCH).as_deref() == Ok("host") {
        println!(
            "cargo:warning={ESCAPE_HATCH}=host: guest image ids will depend on this \
             checkout's path and must not be published"
        );
        println!("cargo:rustc-env={BUILD_MODE}=host");
        risc0_build::embed_methods();
        return;
    }
    println!("cargo:rustc-env={BUILD_MODE}=container");

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

    let guests = guests(&manifest_dir);

    let docker = DockerOptionsBuilder::default()
        // A container inherits nothing from the host, so the build-time configuration
        // the guests read has to be handed over explicitly.
        .env(forwarded_env(&guests))
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

    risc0_build::embed_methods_with_options(
        guests
            .iter()
            .map(|(name, _)| (name.as_str(), options.clone()))
            .collect(),
    );
}

/// The build-time environment the guests read, taken from the guests themselves.
///
/// `option_env!` resolves against the compiler's environment, and the compiler now runs
/// in a container that inherits none of ours, so anything the guests read has to be
/// forwarded. Discovered rather than listed for the reason the package names are: a name
/// held here by hand would be one more copy that can fall out of step, and the failure is
/// quiet -- a guest built without its authority compiles, runs, and refuses every
/// transaction at execution time with an error about a build that configured none.
///
/// Only variables actually set are forwarded, so an unset one still takes `option_env!`'s
/// `None` branch exactly as it does on the host.
fn forwarded_env(guests: &[(String, PathBuf)]) -> Vec<(String, String)> {
    let mut names = BTreeSet::new();
    for (_, dir) in guests {
        read_option_env(&dir.join("src"), &mut names);
    }
    names
        .into_iter()
        .filter_map(|name| {
            println!("cargo:rerun-if-env-changed={name}");
            env::var(&name).ok().map(|value| (name, value))
        })
        .collect()
}

/// Every name a `option_env!` under `dir` reads.
fn read_option_env(dir: &Path, into: &mut BTreeSet<String>) {
    const CALL: &str = "option_env!(\"";
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            read_option_env(&path, into);
        } else if path.extension().is_some_and(|kind| kind == "rs") {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for (at, _) in text.match_indices(CALL) {
                let rest = &text[at + CALL.len()..];
                if let Some(end) = rest.find('"') {
                    into.insert(rest[..end].to_owned());
                }
            }
        }
    }
}

/// The guest packages, read from the metadata `risc0-build` reads to find them.
///
/// Derived rather than listed, because a list here would be a second copy of one that
/// already exists, and the two failing to agree is silent: `risc0-build` applies default
/// options to any guest absent from the map it is handed, and the default is a host
/// build. A fourth entry added to `[package.metadata.risc0]` and not here would compile
/// outside the container and go back to having a directory in its program id, with
/// nothing to say so.
fn guests(manifest_dir: &Path) -> Vec<(String, PathBuf)> {
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
            let dir = manifest_dir.join(path);
            let guest: toml::Value = fs::read_to_string(dir.join("Cargo.toml"))
                .unwrap_or_else(|_| panic!("guest `{path}` has a manifest"))
                .parse()
                .unwrap_or_else(|_| panic!("guest `{path}`'s manifest is TOML"));
            let name = guest
                .get("package")
                .and_then(|package| package.get("name"))
                .and_then(|name| name.as_str())
                .unwrap_or_else(|| panic!("guest `{path}` is a named package"))
                .to_owned();
            (name, dir)
        })
        .collect()
}
