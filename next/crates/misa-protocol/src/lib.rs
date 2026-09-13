//! Both ends of the session protocol, independent of kernels and connections.
pub mod session;
pub mod server;
pub use server::{Server, drive, OUTBOUND_REVISIONS, OutboundBatch};
pub use session::{Emission, Reading, Session};

#[cfg(test)]
mod tests;

pub mod client;
pub use client::Client;
