use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

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
    // Non-empty rather than any particular value, which is how `risc0-build` reads it.
    println!("cargo:rerun-if-env-changed=RISC0_SKIP_BUILD");

    // `risc0-build` compiles no guest at all when this is set and stubs every ELF to
    // empty, so the mode below cannot say `container`: it would be reporting the route
    // a build would have taken rather than the one it took, and
    // `the_guests_were_built_in_a_container` would pass against ELFs carrying no image
    // ids -- which is a stronger version of the case it exists to catch.
    let skipped = !env::var("RISC0_SKIP_BUILD").unwrap_or_default().is_empty();

    if env::var(ESCAPE_HATCH).as_deref() == Ok("host") {
        println!(
            "cargo:warning={ESCAPE_HATCH}=host: guest image ids will depend on this \
             checkout's path and must not be published"
        );
        println!(
            "cargo:rustc-env={BUILD_MODE}={}",
            if skipped { "skipped" } else { "host" }
        );
        risc0_build::embed_methods();
        return;
    }
    println!(
        "cargo:rustc-env={BUILD_MODE}={}",
        if skipped { "skipped" } else { "container" }
    );

    if let Ok(tag) = env::var(TAG_OVERRIDE) {
        assert_eq!(
            tag, GUEST_BUILDER_IMAGE,
            "{TAG_OVERRIDE} is set to `{tag}`, which `risc0-build` would use ahead of \
             the digest this crate pins. A guest built from another image is a different \
             program: its ids are not the ones the ADR records. Unset it, or use \
             {ESCAPE_HATCH}=host if you meant to build outside the container."
        );
    }

    require_buildx(skipped);

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets this"));
    let root = manifest_dir
        .parent()
        .expect("methods/ sits under the workspace root")
        .to_path_buf();

    let guests = guests(&manifest_dir);

    let docker = DockerOptionsBuilder::default()
        // A container inherits nothing from the host, so the build-time configuration
        // the guests read has to be handed over explicitly.
        .env(forwarded_env(&root))
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

/// Refuse early, and by name, when the daemon cannot run the build at all.
///
/// `risc0-build` invokes `docker build --output`, which exists only on BuildKit: a docker
/// install with the legacy builder rejects it as `unknown flag: --output`, several lines
/// above the `docker build failed` that reaches the developer. CI never meets this --
/// GitHub's runners ship buildx -- so the requirement is invisible exactly where it is
/// checked and visible only to whoever builds outside it, which is the first thing that
/// happened when somebody did.
fn require_buildx(skipped: bool) {
    // Nothing will invoke docker when the guest build is skipped, so requiring the
    // plugin there would enforce a dependency for work that does not happen -- and
    // `RISC0_SKIP_BUILD=1 cargo build -p kanon-methods` is a combination this crate's
    // own manifest leans on to explain why `ci.yml` excludes it from `build-test`.
    if skipped {
        return;
    }

    let available = Command::new("docker")
        .args(["buildx", "version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());

    assert!(
        available,
        "`docker buildx` is required to build the guests, and `docker buildx version` \
         did not succeed. `risc0-build` runs `docker build --output`, which the legacy \
         builder rejects; the error it produces names the flag rather than the cause. \
         Install the buildx plugin, or use {ESCAPE_HATCH}=host to build outside the \
         container -- ids from a host build depend on this checkout's path and must not \
         be published."
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
fn forwarded_env(root: &Path) -> Vec<(String, String)> {
    let mut names = BTreeSet::new();
    read_option_env(root, &mut names);
    names
        .into_iter()
        .filter_map(|name| {
            println!("cargo:rerun-if-env-changed={name}");
            env::var(&name).ok().map(|value| (name, value))
        })
        .collect()
}

/// Every name an `option_env!` under `dir` reads.
///
/// Walked from the repository root rather than from each guest's own `src/`, because
/// the root is what `root_dir` copies into the container. A guest that reached its
/// configuration through a workspace crate -- the shape `pull-lib` and `kanon-clock`
/// already have -- would otherwise be forwarded nothing and hit the quiet failure the
/// caller describes.
///
/// The macro is matched by name and the quote found after it, so the whitespace forms
/// `option_env! ("X")` and `option_env!( "X")` are read too. A comment mentioning the
/// macro contributes a name that is forwarded only if it happens to be set, which
/// costs nothing; a name missed does not.
fn read_option_env(dir: &Path, into: &mut BTreeSet<String>) {
    const CALL: &str = "option_env!";
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Build output and history: neither reaches the container, and `target/`
            // alone is large enough to make walking it a real cost.
            let name = entry.file_name();
            if name == "target" || name.to_string_lossy().starts_with('.') {
                continue;
            }
            read_option_env(&path, into);
        } else if path.extension().is_some_and(|kind| kind == "rs") {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for (at, _) in text.match_indices(CALL) {
                let Some(rest) = text[at + CALL.len()..]
                    .trim_start()
                    .strip_prefix('(')
                    .map(str::trim_start)
                    .and_then(|rest| rest.strip_prefix('"'))
                else {
                    continue;
                };
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
