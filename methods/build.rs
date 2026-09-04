use std::{collections::HashMap, env, path::PathBuf};

use risc0_build::{DockerOptionsBuilder, GuestOptionsBuilder};

/// The guest builder image, pinned rather than defaulted.
///
/// `risc0-build`'s own default is `r0.1.88.0`, and taking it would swap the guest
/// compiler under every figure in `COSTS.md`, which were all measured on guest rustc
/// 1.97.0. This tag carries that compiler, so containerising the build changes where a
/// guest is compiled without changing what compiles it. ADR 8's pins and this tag say
/// the same thing and have to move together.
const GUEST_BUILDER_IMAGE: &str = "r0.1.97.0";

/// The guest packages this crate embeds. Every one is named, because `risc0-build`
/// applies default options to a package absent from this map and would silently build
/// that guest on the host.
const GUESTS: [&str; 3] = [
    "kanon-guest",
    "reference-consumer-pull-guest",
    "reference-consumer-aggregator-read-guest",
];

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
    if env::var(ESCAPE_HATCH).as_deref() == Ok("host") {
        println!(
            "cargo:warning={ESCAPE_HATCH}=host: guest image ids will depend on this \
             checkout's path and must not be published"
        );
        risc0_build::embed_methods();
        return;
    }

    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets this"))
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

    risc0_build::embed_methods_with_options(HashMap::from(
        GUESTS.map(|guest| (guest, options.clone())),
    ));
}
