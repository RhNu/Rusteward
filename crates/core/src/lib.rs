//! Pure Rust formatting transformations and source policy checks.

pub mod diagnostic;
pub mod lines;
pub mod rules;
mod source;
pub mod spacing;

pub use ra_ap_syntax::Edition;

/// Generated sources are outside the authored-source workflow.
pub fn is_generated(source: &str) -> bool {
    source
        .lines()
        .take(5)
        .any(|line| line.contains("@generated"))
}
