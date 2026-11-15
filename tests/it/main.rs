//! Integration tests: exercise calamus through its public API only.
//!
//! Everything lives in one test binary (faster to build than one per file).
//! To cover a new module, add `tests/it/<module>.rs` and a `mod <module>;` below.
//! Unit tests for private internals stay in `#[cfg(test)] mod tests` inside `src/`.
//!
//! Run all:          cargo test
//! Run one module:   cargo test --test it buffer::

mod common;

mod buffer;
// mod display;
mod layout;
// mod units;
