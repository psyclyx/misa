use misa_tui::{Session, run, workspace::Workspace};
use std::io::{BufRead, IsTerminal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut force_print = false;
    let mut positional = Vec::new();
    let mut targets = Vec::new();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--print" | "-p" => force_print = true,
            "--daemon" | "-d" => targets.push(arguments.next().ok_or("--daemon needs an address or pairing ticket")?),
            "--help" | "-h" => {
                println!("usage: misa [--print|-p] [--daemon ADDRESS]... [misa:<endpoint>:<session>] [prompt]\nWithout an address, discovers local daemons. /daemon selects a daemon; /session selects its session.");
                return Ok(());
            }
            _ => positional.push(argument),
        }
    }
    if positional.first().is_some_and(|s: &String| s.starts_with("misa:") || s.starts_with("misa-pair:")) {
        targets.push(positional.remove(0));
    }
    if positional.len() > 1 { return Err("Pass the prompt as one quoted argument".into()); }
    let mut remote = Workspace::start(&targets).await?;
    if misa_tui::print::interactive(
        force_print,
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    ) {
        if let Some(prompt) = positional.first() {
            remote
                .send(misa_kit::intent::Intent::Prompt {
                    text: prompt.clone(),
                    attachments: Vec::new(),
                })
                .await?;
        }
        run(&mut remote).await?;
    } else {
        let (sender, receiver) = tokio::sync::mpsc::channel(32);
        let prompt = positional.first().cloned();
        std::thread::spawn(move || {
            if let Some(prompt) = prompt {
                if sender.blocking_send(Ok(prompt)).is_err() {
                    return;
                }
            }
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
