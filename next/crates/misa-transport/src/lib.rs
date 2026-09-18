//! Scoped daemon connections, admission, local discovery, and blob transport.
pub mod admission;
pub mod blob;
pub mod identity;
pub mod iroh;
#[cfg(unix)]
pub mod local;
pub mod pairing;
pub mod scoped_client;
mod scoped_invocations;
pub mod scoped_io;
#[cfg(test)]
mod scoped_network_tests;
pub mod scoped_server;
pub mod server;
