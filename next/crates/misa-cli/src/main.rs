//! A headless client: attach to a session and print what it says.
//!
//! ```sh
//! misa misa:<endpoint id>:<session>             # print the view as it changes
//! misa misa:<endpoint id>:<session> "a prompt"  # then submit one and follow along
//! ```
//!
//! This is the fourth frontend and the least interesting one: it proves that a client
//! needs no presentation vocabulary of its own to be useful, which is what "the UI is
//! data and the client draws it" has to mean to be testable. It is also what a script
//! uses, so it prints only when the rendered text actually changed.

use misa_proto::wire::Intent;
use misa_proto::{Query, SessionMsg, SubId};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let Some(ticket) = arguments.next() else {
        eprintln!("usage: misa misa:<endpoint id>:<session> [prompt]");
        return Ok(());
    };
    let prompt = arguments.next();
    // A ticket, or a pairing string: whatever the daemon printed or the QR said. A client that
    // is not yet paired says so with its code before it says hello.
    let (parsed, code) = misa_proto::Pairing::given(&ticket)?;
    let endpoint = misa_net::iroh::bind_for(&parsed.node).await?;
    let target = misa_net::iroh::address_of(&parsed.node)?;
    if let Some(code) = &code {
        let message = misa_net::iroh::Client::pair(&endpoint, target.clone(), code, "the command line").await?;
        eprintln!("[paired] {message}");
    }
    let info = misa_proto::ClientInfo::new("misa-cli", env!("CARGO_PKG_VERSION"));
    let mut client = misa_net::iroh::Client::connect(&endpoint, target, info, &parsed.session).await?;
    client
        .subscribe(SubId(1), Query::new(misa_proto::VIEW_QUERY))
        .await?;
    // The declarations arrive with the session, and they are what makes this frontend as
    // capable as any other: a client that holds them knows that `/models` exists and that
    // `/model` wants a model, with no round trip and no second source of truth. Sending them
    // out empty — which is what this did before — is a client that cannot type a command.
    let commands = client.session().map(|session| session.commands.clone()).unwrap_or_default();
    if let Some(prompt) = prompt {
        client
            .intent(1, Intent::Prompt { text: prompt, attachments: Vec::new() })
            .await?;
    }

    // Reading standard input is what makes this usable in a pipeline: a line becomes a
    // prompt, and end-of-file ends the session. A thread rather than a `select!` because
    // stdin is blocking and the transport is not.
    let (lines_in, mut lines) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        use std::io::BufRead as _;
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(line) => {
                    if lines_in.send(line).is_err() {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    });

    let theme = misa_render::Theme::plain();
    let mut printed = String::new();
    let mut next_intent = 2u64;
    loop {
        tokio::select! {
            message = client.next() => match message? {
                Some(SessionMsg::View { view, .. }) => {
                    let text = misa_render::to_plain(&misa_render::render(&view, &theme, 100));
                    if text != printed {
                        print!("{text}");
                        use std::io::Write as _;
                        std::io::stdout().flush().ok();
                        printed = text;
                    }
                }
                Some(SessionMsg::Event { event, .. }) => {
                    if let misa_proto::SessionEvent::Notice { level, text } = event {
                        eprintln!("[{}] {text}", level.as_str());
                    }
                }
                Some(SessionMsg::Fault { fault, .. }) => eprintln!("[{}] {}", fault.code, fault.message),
                Some(_) => {}
                None => return Ok(()),
            },
            line = lines.recv() => match line {
                Some(line) if !line.trim().is_empty() => {
                    let intent = match misa_client::intent::parse(&line, &commands) {
                        // With no declarations in hand this frontend sends what it was
                        // given as a prompt; a richer client would hold them and offer a
                        // picker, which is the whole point of the declaration.
                        misa_client::intent::Parsed::Prompt(text) => Intent::Prompt { text, attachments: Vec::new() },
                        misa_client::intent::Parsed::Command { name, args } => Intent::Command { name, args },
                        other => {
                            eprintln!("[not sent] {other:?}");
                            continue;
                        }
                    };
                    client.intent(next_intent, intent).await?;
                    next_intent += 1;
                }
                Some(_) => {}
                // End of input ends the client. The session keeps running, which is what
                // detaching means.
                None => return Ok(()),
            },
        }
    }
}
