use misa_tui::{Remote, Session, run};
use std::io::{BufRead, IsTerminal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut force_print = false;
    let mut positional = Vec::new();
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--print" | "-p" => force_print = true,
            "--help" | "-h" => {
                println!("usage: misa [--print|-p] misa:<endpoint id>:<session> [prompt]");
                return Ok(());
            }
            _ => positional.push(argument),
        }
    }
    if positional.is_empty() || positional.len() > 2 {
        return Err("usage: misa [--print|-p] misa:<endpoint id>:<session> [prompt]".into());
    }
    let mut remote = Remote::attach(&positional[0]).await?;
    if let Some(prompt) = positional.get(1) {
        remote
            .send(misa_proto::wire::Intent::Prompt {
                text: prompt.clone(),
                attachments: Vec::new(),
            })
            .await?;
    }
    if misa_tui::print::interactive(
        force_print,
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    ) {
        run(&mut remote).await?;
    } else {
        let (sender, receiver) = tokio::sync::mpsc::channel(32);
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                if sender
                    .blocking_send(line.map_err(|error| error.to_string()))
                    .is_err()
                {
                    break;
                }
            }
        });
        misa_tui::print::run(
            &mut remote,
            receiver,
            &mut std::io::stdout(),
            &mut std::io::stderr(),
        )
        .await?;
    }
    Ok(())
}
