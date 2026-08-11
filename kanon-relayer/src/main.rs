//! Headless RedStone-to-LEZ relayer daemon.
//!
//! # Not yet implemented
//!
//! Nothing here yet. The daemon comprises a Logos Core skeleton, the DDL fetch
//! over parallel gateways with first-success selection, heartbeat and deviation
//! triggers, retry and back-off, structured logging with reject reasons,
//! wallet-balance monitoring, and clean shutdown with crash recovery.
//! `TRACEABILITY.md` maps each to the requirement it satisfies and the task that
//! delivers it.

fn main() {
    eprintln!(
        "kanon-relayer is a layout placeholder; the daemon is not implemented yet. \
         See TRACEABILITY.md."
    );
    std::process::exit(1);
}
