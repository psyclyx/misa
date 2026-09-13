//! Serve one session to a browser.
//!
//! ```sh
//! # the browser's server is a client of an agent daemon
//! misa-web --ticket misa:<endpoint id>:<session> --listen 127.0.0.1:8080
//! # or a session in this process, which needs no daemon at all
//! misa-web --local --listen 127.0.0.1:8080
//! ```
//!
//! The two differ in one place: where the view arrives from. Everything a browser sees —
//! the document, the region, the stream — is produced by the same renderer.

use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let mut ticket = None;
    let mut local = false;
    let mut listen = "127.0.0.1:8080".to_string();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--ticket" | "-t" => ticket = arguments.next(),
            "--local" => local = true,
            "--listen" | "-l" => listen = arguments.next().unwrap_or(listen),
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }
    let address: std::net::SocketAddr = listen.parse()?;

    match (local, ticket) {
        (true, _) => {
            let (runtime, blobs) = local_runtime();
            misa_web::serve_with(runtime, blobs, address).await?
        }
        (false, Some(ticket)) => misa_web::attach(&ticket, address).await?,
        (false, None) => eprintln!(
            "a browser frontend needs a session: pass --ticket misa:<endpoint id>:<session>, or --local"
        ),
    }
    Ok(())
}

/// A session in this process, with the scripted provider, and the store its images live in.
///
/// The zero-setup case: no daemon, no account, no network. It is also what the frontend's own
/// tests drive, so the path a person clicks through is the path that is tested.
///
/// The store is named here rather than left to the kernel, because the same one has to be
/// reachable from the browser's side: `/attach` puts a file in it and `<img src="/blob/…">`
/// reads it back out, and two stores would make a picture that is there and cannot be seen.
fn local_runtime() -> (Arc<misa_session::Runtime>, Arc<misa_web::Source>) {
    use misa_kernel::{Kernel, LocalKernel, Provider, ScriptedProvider, Turn};
    let provider: Arc<dyn Provider> = ScriptedProvider::new([
        Turn::call("echo", misa_value::Value::str("hello from the browser"), Turn::say(
            "Here is what I found:\n\n```rust\nlet answer = 42;\n```\n\nAsk me anything else.",
        )),
        Turn::say("Here is what I found:\n\n```rust\nlet answer = 42;\n```\n\nAsk me anything else."),
        Turn::say("Still here."),
    ]);
    let blobs = Arc::new(misa_kernel::Blobs::in_memory());
    let kernel: Arc<dyn Kernel> = Arc::new(LocalKernel::new(provider).with_blobs(blobs.clone()));
    let runtime = misa_session::Runtime::start(
        "local",
        "a local session (scripted)",
        Some("local".into()),
        kernel,
        "scripted",
        "scripted-1",
        misa_value::Value::Null,
    );
    (runtime, Arc::new(misa_web::Source::Local(blobs)))
}
