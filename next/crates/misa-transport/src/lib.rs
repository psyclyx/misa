//! Scoped daemon connections, admission, local discovery, and blob transport.
pub mod admission;
pub mod pairing;
pub mod blob;
pub mod iroh;
pub mod server;
pub mod scoped_server;
pub mod identity;
#[cfg(unix)]
pub mod local;
pub mod scoped_io;
pub mod scoped_client;
#[cfg(test)]
mod scoped_network_tests;
// Temporary protocol-only compatibility coverage; no legacy socket API is exported.
#[cfg(test)]
mod legacy_tests;
