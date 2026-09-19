//! Pipeline mode, with injected input, output and session for deterministic tests.
use crate::Session;
use misa_kit::intent::Intent;
use misa_kit::intent::{Parsed, parse};
use std::io::Write;

pub fn interactive(force_print: bool, stdin_tty: bool, stdout_tty: bool) -> bool {
    !force_print && stdin_tty && stdout_tty
}

/// Read prompts serially and drain the submitted turn before detaching on EOF.
pub async fn run(
    session: &mut dyn Session,
    mut input: tokio::sync::mpsc::Receiver<Result<String, String>>,
    output: &mut dyn Write,
    errors: &mut dyn Write,
) -> Result<(), String> {
    let mut printed = String::new();
    let mut ready = false;
    let mut latest = None;
    let mut staged: Option<String> = None;
    let mut pending: Option<std::collections::BTreeSet<String>> = None;
    let mut settled = std::collections::BTreeSet::new();
    loop {
        if ready && pending.is_none() {
            if let Some(line) = staged.take() {
                if let Some(request) = crate::save::parse(&line) {
                    let result =
                        match request {
                            Ok(request) => match latest
                                .as_ref()
                                .ok_or("No view received yet")
                                .and_then(|view| {
                                    crate::save::target(view, &request)
                                        .map_err(|_| "No matching attachment")
                                }) {
                                Ok(node) => session
                                    .save_attachment(node, &request.destination)
                                    .await
                                    .map(|_| request.destination),
                                Err(error) => Err(error.into()),
                            },
                            Err(error) => Err(error),
                        };
                    match result {
                        Ok(path) => writeln!(errors, "Saved {path}"),
                        Err(error) => writeln!(errors, "[not saved] {error}"),
                    }
                    .map_err(|error| error.to_string())?;
                    continue;
                }

                let commands = session.catalog().commands;
                let intent = match parse(&line, &commands) {
                    Parsed::Prompt(text) => Intent::Prompt {
                        text,
                        attachments: Vec::new(),
                    },
                    Parsed::Command { name, args } => Intent::Command { name, args },
                    other => {
                        writeln!(errors, "[not sent] {other:?}")
                            .map_err(|error| error.to_string())?;
                        continue;
                    }
                };
                if matches!(intent, Intent::Prompt { .. }) {
                    pending = Some(settled.clone());
                    if session.turn_settled().is_some() {
                        printed.clear();
                    }
                }
                session.send(intent).await?;
            }
        }
        tokio::select! {
            biased;
            view = session.next() => {
                let Some(view) = view? else { return Ok(()); };
                ready = true;
                settled = settled_messages(&view);
                let text = misa_render::to_plain(&misa_render::render(&view, &misa_render::Theme::plain(), 100));
                if text != printed {
                    // Append-only renderings need only their new suffix. Structural changes
                    // still print the replacement until the transport carries view patches.
                    let suffix = text.strip_prefix(&printed).unwrap_or(&text);
                    output.write_all(suffix.as_bytes()).and_then(|_| output.flush()).map_err(|error| error.to_string())?;
                    printed = text;
                }
                latest = Some(view.clone());
                if session.turn_settled().unwrap_or_else(|| pending.as_ref().is_some_and(|before| settled.difference(before).next().is_some()) && !working(&view)) {
                    pending = None;
                }
            }
            line = input.recv(), if staged.is_none() && pending.is_none() => {
                let Some(line) = line else { return Ok(()); };
                let line = line?;
                if !line.trim().is_empty() { staged = Some(line); }
            }
        }
    }
}

fn settled_messages(view: &misa_proto::view::Node) -> std::collections::BTreeSet<String> {
    use misa_proto::view::State;
    let mut ids = std::collections::BTreeSet::new();
    fn visit(node: &misa_proto::view::Node, ids: &mut std::collections::BTreeSet<String>) {
        if node.role == "message.assistant"
            && matches!(
                node.state,
                Some(State::Done | State::Failed | State::Cancelled)
            )
        {
            ids.insert(node.id.clone());
        }
        for child in &node.children {
            visit(child, ids);
        }
    }
    visit(view, &mut ids);
    ids
}

fn working(view: &misa_proto::view::Node) -> bool {
    view.actions.iter().any(|action| action.id == "turn.cancel")
        || view.children.iter().any(working)
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::view::{Choice, Node};
    struct Fake {
        views: std::collections::VecDeque<Node>,
        sent: Vec<Intent>,
        wait: bool,
    }
    #[async_trait::async_trait]
    impl Session for Fake {
        async fn next(&mut self) -> Result<Option<Node>, String> {
            tokio::task::yield_now().await;
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
            self.views.push_back(
                Node::text(
                    "message.assistant",
                    [misa_proto::view::Span::plain("delayed answer")],
                )
                .id(format!("reply.{}", self.sent.len()))
                .state(misa_proto::view::State::Done),
            );
            Ok(())
        }
        async fn complete(&mut self, _: &str, _: &str) -> Result<(Vec<Choice>, bool), String> {
            unreachable!()
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
    async fn eof_before_delayed_reply_drains_the_submitted_prompt() {
        let mut session = Fake {
            views: [Node::section("initial")].into(),
            sent: Vec::new(),
            wait: true,
        };
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        sender.send(Ok("  ".into())).await.unwrap();
        sender.send(Ok("héllo".into())).await.unwrap();
        drop(sender);
        let mut output = Vec::new();
        run(&mut session, receiver, &mut output, &mut Vec::new())
            .await
            .unwrap();
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("delayed answer")
        );
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
            views: [Node::section("initial")].into(),
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
    #[tokio::test]
    async fn empty_input_detaches_without_waiting_for_a_view() {
        let mut session = Fake {
            views: [].into(),
            sent: Vec::new(),
            wait: true,
        };
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        drop(sender);
        run(&mut session, receiver, &mut Vec::new(), &mut Vec::new())
            .await
            .unwrap();
        assert!(session.sent.is_empty());
    }

    #[tokio::test]
    async fn a_finite_pipeline_drains_each_prompt() {
        let mut session = Fake {
            views: [Node::section("initial")].into(),
            sent: Vec::new(),
            wait: true,
        };
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        sender.send(Ok("one".into())).await.unwrap();
        sender.send(Ok("two".into())).await.unwrap();
        drop(sender);
        run(&mut session, receiver, &mut Vec::new(), &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(session.sent.len(), 2);
        assert!(session.views.is_empty(), "the last reply was not drained");
    }
}
