//! The three modes, one module each.
//!
//! All three are stubs at phase 00. The scaffold exists so that the dispatch,
//! the portable path resolution and the config round-trip are under test before
//! any window code lands -- the overlay (phase 01) is the riskiest part of the
//! port, and it should be built on something already known to work.

pub mod overlay;
pub mod settings;
pub mod setup;
