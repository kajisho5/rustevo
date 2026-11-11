//! rustevo: apps that agents evolve, with a replayable record of every change.
pub mod carry;
pub mod gen;
pub mod lang;
pub mod plan;
pub mod runtime;
pub mod session;
pub mod site;
pub mod wasm;

#[cfg(test)]
mod tests;
