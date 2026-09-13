//! Pipeline mode, with injected input, output and session for deterministic tests.
use crate::Session;
use misa_client::intent::{Parsed, parse};
use misa_proto::wire::Intent;
use std::io::Write;

pub fn interactive(force_print: bool, stdin_tty: bool, stdout_tty: bool) -> bool {
    !force_print && stdin_tty && stdout_tty
}

/// EOF detaches; the session continues independently.
pub async fn run(
    session: &mut dyn Session,
    mut input: tokio::sync::mpsc::Receiver<Result<String, String>>,
    output: &mut dyn Write,
    errors: &mut dyn Write,
) -> Result<(), String> {
    let mut printed = String::new();
    loop {
        tokio::select! {
            view = session.next() => {
                let Some(view) = view? else { return Ok(()); };
                let text = misa_render::to_plain(&misa_render::render(&view, &misa_render::Theme::plain(), 100));
                if text != printed {
                    output.write_all(text.as_bytes()).and_then(|_| output.flush()).map_err(|error| error.to_string())?;
                    printed = text;
                }
            }
            line = input.recv() => {
                let Some(line) = line else { return Ok(()); };
                let line = line?;
                if line.trim().is_empty() { continue; }
                let commands = session.info().map(|info| info.commands).unwrap_or_default();
                let intent = match parse(&line, &commands) {
                    Parsed::Prompt(text) => Intent::Prompt { text, attachments: Vec::new() },
                    Parsed::Command { name, args } => Intent::Command { name, args },
                    other => {
                        writeln!(errors, "[not sent] {other:?}").map_err(|error| error.to_string())?;
                        continue;
                    }
                };
                session.send(intent).await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{
        view::{Choice, Node},
        wire::SessionInfo,
    };
    struct Fake {
        views: std::collections::VecDeque<Node>,
        sent: Vec<Intent>,
        wait: bool,
    }
    #[async_trait::async_trait]
    impl Session for Fake {
        async fn next(&mut self) -> Result<Option<Node>, String> {
            if let Some(view) = self.views.pop_front() {
                return Ok(Some(view));
            }
            if self.wait {
                std::future::pending().await
            } else {
                Ok(None)
            }
        }
        async fn send(&mut self, intent: Intent) -> Result<(), String> {
            self.sent.push(intent);
            Ok(())
        }
        async fn complete(&mut self, _: &str, _: &str) -> Result<(Vec<Choice>, bool), String> {
            unreachable!()
        }
        fn info(&self) -> Option<SessionInfo> {
            None
        }
    }
    #[test]
    fn only_two_terminals_enable_interaction() {
        for force in [false, true] {
            for input in [false, true] {
                for output in [false, true] {
                    assert_eq!(interactive(force, input, output), !force && input && output);
                }
            }
        }
    }
    #[tokio::test]
    async fn identical_views_print_once_and_disconnect_finishes() {
        let view = Node::text("message", [misa_proto::view::Span::plain("hello")]);
        let mut session = Fake {
            views: [view.clone(), view].into(),
            sent: Vec::new(),
            wait: false,
        };
        let (_sender, receiver) = tokio::sync::mpsc::channel(1);
        let mut output = Vec::new();
        run(&mut session, receiver, &mut output, &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap().matches("hello").count(),
            1
        );
    }
    #[tokio::test]
    async fn input_sends_prompts_ignores_blank_lines_and_eof_detaches() {
        let mut session = Fake {
            views: [].into(),
            sent: Vec::new(),
            wait: true,
        };
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        sender.send(Ok("  ".into())).await.unwrap();
        sender.send(Ok("héllo".into())).await.unwrap();
        drop(sender);
        run(&mut session, receiver, &mut Vec::new(), &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(
            session.sent,
            [Intent::Prompt {
                text: "héllo".into(),
                attachments: Vec::new()
            }]
        );
    }
    #[tokio::test]
    async fn input_errors_propagate() {
        let mut session = Fake {
            views: [].into(),
            sent: Vec::new(),
            wait: true,
        };
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        sender.send(Err("broken input".into())).await.unwrap();
        assert_eq!(
            run(&mut session, receiver, &mut Vec::new(), &mut Vec::new()).await,
            Err("broken input".into())
        );
    }
    #[tokio::test]
    async fn output_failure_is_reported() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("broken output"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let view = Node::text("message", [misa_proto::view::Span::plain("hello")]);
        let mut session = Fake {
            views: [view].into(),
            sent: Vec::new(),
            wait: false,
        };
        let (_sender, receiver) = tokio::sync::mpsc::channel(1);
        assert_eq!(
            run(&mut session, receiver, &mut Broken, &mut Vec::new()).await,
            Err("broken output".into())
        );
    }
}
