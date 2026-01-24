//! E2E test harness for CTX.
//!
//! This module contains test infrastructure with intentionally unused builders,
//! variants, and methods that will be used as more e2e scenarios are written.

#![allow(dead_code)]

pub mod assertions;
pub mod clock;
pub mod runner;
pub mod scenario;
pub mod steps;
pub mod workspace;

// Re-export commonly used types
pub use assertions::{Assertion, SessionStateMatch};
pub use scenario::Scenario;
