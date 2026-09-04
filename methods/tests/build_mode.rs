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

/// The one case that actually costs something: publishing an id from a host build.
///
/// Pinning the ids themselves would fail on every legitimate change to a guest or to
/// anything it links, which is churn for a property that is not what went wrong. This
/// fails only when the build was not containerised.
#[test]
fn the_guests_were_built_in_a_container() {
    assert_eq!(
        kanon_methods::GUEST_BUILD_MODE,
        "container",
        "these guests were built on the host, so their image ids depend on the path \
         this checkout sits at and are not the ids `[M3-08:02]` records. That is what \
         `KANON_GUEST_BUILD=host` buys, and it is fine for a local build on an \
         architecture the builder image cannot serve -- but unset it before trusting \
         an id, and never publish one measured this way"
    );
}
