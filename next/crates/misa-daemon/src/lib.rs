//! Daemon-owned state and routing, composed independently of the command line.
pub mod archive;
pub mod directory;

pub mod lifecycle;

pub mod delegation;
pub mod membership;

#[cfg(test)]
mod presence_tests;
