use misa_tui::{Session, run, workspace::Workspace};
use std::io::{BufRead, IsTerminal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut force_print = false;
    let mut start_local = false;
    let mut positional = Vec::new();
    let mut targets = Vec::new();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--print" | "-p" => force_print = true,
            "--start-local" => start_local = true,
            "--daemon" | "-d" => targets.push(
                arguments
                    .next()
                    .ok_or("--daemon needs an address or pairing ticket")?,
            ),
            "--help" | "-h" => {
                println!(
                    "usage: misa [--print|-p] [--start-local] [--daemon ADDRESS]... [misa:<endpoint>:<session>] [prompt]\nWithout an address, discovers local daemons. --start-local starts one only if none is live; /daemon selects a daemon; /session selects its session."
                );
                return Ok(());
            }
            _ => positional.push(argument),
        }
    }
    if positional
        .first()
        .is_some_and(|s: &String| s.starts_with("misa:") || s.starts_with("misa-pair:"))
    {
        targets.push(positional.remove(0));
    }
    if positional.len() > 1 {
        return Err("Pass the prompt as one quoted argument".into());
    }
    if start_local {
        validate_start_local(&targets)?;
        #[cfg(unix)]
        misa_local_start::discover_or_start().await?;
        #[cfg(not(unix))]
        return Err("--start-local requires Unix".into());
    }
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

fn validate_start_local(targets: &[String]) -> Result<(), &'static str> {
    if !targets.is_empty() {
        return Err("--start-local cannot be used with an explicit daemon target");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_start_local;
    #[test]
    fn cold_start_refuses_explicit_targets() {
        assert!(validate_start_local(&[]).is_ok());
        for target in ["misa:node:session", "misa-pair:node:code", "node"] {
            assert!(validate_start_local(&[target.into()]).is_err());
        }
    }
}
