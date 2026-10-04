//! The guests in this build were compiled in a container, and not on this host.
//!
//! A program's id *is* its guest image id, and outside a container that id depends on
//! the absolute path this repository sits at — so a host build produces ids that are
//! nobody else's, for a program whose accounts are PDAs derived from them. `[M3-08:02]`
//! has the measurements.
//!
//! `KANON_GUEST_BUILD=host` is a supported escape for architectures the builder image
//! cannot serve, and it warns. This exists because a warning is the wrong instrument: it
//! is lost in a CI log, and what it is announcing is that the ids in this build are not
//! the ones the ADR records.
//!
//! Runs no guest, so it costs nothing to keep.

/// The one case that actually costs something: publishing an id that is not a
/// containerised guest's.
///
/// Pinning the ids themselves would fail on every legitimate change to a guest or to
/// anything it links, which is churn for a property that is not what went wrong. This
/// fails only when the build was not containerised, which covers `KANON_GUEST_BUILD=host`
/// and `RISC0_SKIP_BUILD`: the second compiles no guest at all and stubs every ELF to
/// empty, so an id read from one of those belongs to no guest.
#[test]
fn the_guests_were_built_in_a_container() {
    assert_eq!(
        kanon_methods::GUEST_BUILD_MODE,
        "container",
        "these guests were not built in the container: the build reports `{}`. `host` \
         means their image ids depend on the path this checkout sits at, which is what \
         `KANON_GUEST_BUILD=host` buys and is fine for a local build on an architecture \
         the builder image cannot serve. `skipped` means `RISC0_SKIP_BUILD` was set and \
         no guest was compiled at all, so the ELFs behind these ids are empty. Either \
         way, unset it before trusting an id, and never publish one measured this way",
        kanon_methods::GUEST_BUILD_MODE
    );
}
