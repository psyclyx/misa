//! Open a pixel window, or export a session view as PNG.
//!
//! ```sh
//! # a session, live
//! misa-skia --ticket misa:<endpoint id>:<session> --out frame.png
//! # a view that was saved, with no session at all
//! misa-skia --view frame.json --out frame.png
//! # keep rendering as it changes
//! misa-skia --ticket misa:<endpoint id>:<session> --out frame.png --every-ms 500
//! ```
//!
//! The file is the useful half of a pixel frontend: it can be checked in a test, produced
//! on a machine with no display, and diffed against last week's. A window is a second
//! consumer of the same scene and needs nothing new from a session.

use std::sync::Arc;
use std::time::Duration;

use misa_proto::view::Node;
use misa_proto::{Query, SessionMsg, SubId};

fn usage() -> String {
    "usage: misa-skia (--ticket misa:<endpoint id>:<session> | --view view.json) [--window] [--out frame.png] [--every-ms N] [--columns N]"
        .into()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut window = false;
    let mut ticket = None;
    let mut view_file = None;
    let mut out = None;
    let mut every_ms: Option<u64> = None;
    let mut columns = 100u32;
    let mut rows = 40u32;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--window" => window = true,
            "--ticket" | "-t" => ticket = arguments.next(),
            "--view" | "-v" => view_file = arguments.next(),
            "--out" | "-o" => out = arguments.next(),
            "--every-ms" => every_ms = arguments.next().and_then(|value| value.parse().ok()),
            "--columns" => columns = arguments.next().and_then(|value| value.parse().ok()).unwrap_or(columns),
            "--rows" => rows = arguments.next().and_then(|value| value.parse().ok()).unwrap_or(rows),
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }
    if ticket.is_none() && view_file.is_none() {
        return Err(usage().into());
    }

    let mut first: Option<Node> = match &view_file {
        Some(path) => {
            let text = std::fs::read_to_string(path)?;
            Some(serde_json::from_str(&text)?)
        }
        None => None,
    };

    if window || out.is_none() {
        let view = first.take().unwrap_or_else(|| Node::section("connecting"));
        misa_skia::window::run(view, ticket, out)?;
        return Ok(());
    }
    let out = out.expect("PNG output selected");

    if let Some(text) = first.take() {
        write_frame(&text, columns, rows, &out)?;
        println!("wrote {out}");
        if every_ms.is_none() {
            return Ok(());
        }
    }

    let Some(ticket) = ticket else {
        return Ok(());
    };
    // A ticket, or a pairing string: whatever the daemon printed or the QR said.
    let (parsed, code) = misa_proto::Pairing::given(&ticket)?;
    let endpoint = misa_net::iroh::bind_for(&parsed.node).await?;
    let target = misa_net::iroh::address_of(&parsed.node)?;
    if let Some(code) = &code {
        let message = misa_net::iroh::Client::pair(&endpoint, target.clone(), code, "the pixel frontend").await?;
        eprintln!("[paired] {message}");
    }
    let info = misa_proto::ClientInfo::new("misa-skia", env!("CARGO_PKG_VERSION"));
    // `Arc` because a reconnect needs the endpoint again; there is exactly one here and
    // the client takes it by reference.
    let endpoint = Arc::new(endpoint);
    let mut client = misa_net::iroh::Client::connect(&endpoint, target, info, &parsed.session).await?;
    client
        .subscribe(SubId(1), Query::new(misa_proto::VIEW_QUERY))
        .await?;

    let mut current = misa_proto::sync::ClientView::default();
    loop {
        let Some(message) = client.next().await? else { return Ok(()) };
        match current.receive(&message) {
            Ok(true) => if let Some(view) = current.rendered() {
                write_frame(&view, columns, rows, &out)?;
                println!("wrote {out}");
                if every_ms.is_none() { return Ok(()); }
            },
            Err(_) => client.subscribe(SubId(1), Query::new(misa_proto::VIEW_QUERY)).await?,
            Ok(false) => if let SessionMsg::Fault { fault, .. } = message { eprintln!("[{}] {}", fault.code, fault.message); },
        }
        if let Some(every) = every_ms {
            // A frame is a snapshot: the loop waits for the next change rather than
            // burning a core redrawing an identical scene.
            tokio::time::sleep(Duration::from_millis(every.max(16))).await;
        }
    }
}

/// Paint a scene to a file.
fn write_frame(view: &Node, columns: u32, rows: u32, out: &str) -> Result<(), Box<dyn std::error::Error>> {
    let theme = misa_render::Theme::dark();
    let scene = misa_skia::scene(view, &theme, columns as usize, rows as usize, misa_skia::Layout::default());
    let png = misa_skia::paint::png(&scene, misa_render::Color::Rgb(20, 22, 26))?;
    std::fs::write(out, png)?;
    Ok(())
}
