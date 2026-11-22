//! Shell integration: scan PTY output for OSC 133 / 6973 / 7 marks, turn them into blocks,
//! and inject the integration scripts into supported shells.

mod inject;
mod scanner;
mod tracker;

pub use inject::{Injection, inject, write_scripts};
pub use scanner::{Mark, ScanEvent, Scanner};
pub use tracker::{BlockTracker, TrackerEvent};
