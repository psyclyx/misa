//! Attach to a session and drive the terminal frontend.
//!
//! ```sh
//! misa-tui misa:<endpoint id>:<session>
//! ```
//!
//! A ticket is all it takes, because a session is addressed rather than configured:
//! the same client reaches a session on this machine or on another one, and nothing
//! about how it draws changes.

use misa_tui::{Remote, run};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let Some(ticket) = arguments.next() else {
        eprintln!("usage: misa-tui misa:<endpoint id>:<session>");
        return Ok(());
    };
    let mut remote = Remote::attach(&ticket).await?;
    run(&mut remote).await?;
    Ok(())
}
