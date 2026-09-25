//! A terminal destination is local state; only the selected attachment action crosses the wire.
use misa_proto::view::Node;

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub number: Option<usize>,
    pub destination: String,
}

pub fn parse(line: &str) -> Option<Result<Request, String>> {
    // Match the composer's single-line command boundary; pasted multi-line
    // messages remain prompts, not local file operations.
    if line.contains('\n') {
        return None;
    }
    let rest = line.trim().strip_prefix("/save")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim();
    if rest.is_empty() {
        return Some(Err(
            "Use /save [attachment number] <local path>; omit the number for the latest attachment"
                .into(),
        ));
    }
    let (number, destination) = match rest.split_once(char::is_whitespace) {
        Some((first, path)) if first.parse::<usize>().is_ok() => (first.parse().ok(), path.trim()),
        _ => (None, rest),
    };
    if number == Some(0) || destination.is_empty() {
        return Some(Err(
            "Attachment numbers start at 1; a local path is required".into(),
        ));
    }
    Some(Ok(Request {
        number,
        destination: destination.to_string(),
    }))
}

pub fn attachments(view: &Node) -> Vec<&Node> {
    fn visit<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
        if node
            .actions
            .iter()
            .any(|action| action.id == "attachment.save")
        {
            out.push(node);
        }
        for child in &node.children {
            visit(child, out);
        }
    }
    let mut nodes = vec![];
    visit(view, &mut nodes);
    nodes
}

pub fn target<'a>(view: &'a Node, request: &Request) -> Result<&'a str, String> {
    let attachments = attachments(view);
    let index = request.number.unwrap_or(attachments.len());
    attachments
        .get(index.saturating_sub(1))
        .map(|node| node.id.as_str())
        .ok_or_else(|| "No attachment with that number is in this view".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_destinations_and_attachment_selection_are_not_commands_sent_to_session() {
        assert_eq!(
            parse("/save 2 /tmp/my file.png").unwrap().unwrap(),
            Request {
                number: Some(2),
                destination: "/tmp/my file.png".into()
            }
        );
        assert_eq!(parse("/save ./photo.png").unwrap().unwrap().number, None);
        assert!(parse("/save").unwrap().is_err());
        assert!(parse("/save 0 x").unwrap().is_err());
        assert!(parse("/saved").is_none());
        assert!(parse("/save /tmp/path\nmore text").is_none());
        assert_eq!(
            parse("  /save /tmp/path  ").unwrap().unwrap().destination,
            "/tmp/path"
        );
    }

    #[test]
    fn connected_host_declares_save_but_never_shadows_a_session_save() {
        use crate::{Catalog, Key, KeyOut, Screen};
        use misa_kit::intent::{Command, Intent};

        let mut screen = Screen::new(80, 24);
        screen.declare(&Catalog::default());
        assert!(
            screen
                .command_candidates()
                .iter()
                .any(|item| item.value == "/save")
        );
        screen.editor.set_text("/save 2 /tmp/my file.png");
        assert_eq!(
            screen.key(Key::Submit),
            KeyOut::Submitted("/save 2 /tmp/my file.png".into())
        );
        assert!(screen.editor.is_empty());

        screen.declare(&Catalog {
            commands: vec![Command::new("save", "Session save", "session-owned")],
            sources: vec![],
        });
        assert_eq!(
            screen
                .command_candidates()
                .iter()
                .filter(|item| item.value == "/save")
                .count(),
            1
        );
        screen.editor.set_text("/save");
        assert!(
            matches!(screen.key(Key::Submit), KeyOut::Intent(Intent::Command { name, .. }) if name == "save")
        );
        screen.editor.set_text("/save /tmp/local");
        assert_eq!(screen.key(Key::Submit), KeyOut::Local);
        assert_eq!(screen.editor.text(), "/save /tmp/local");
    }
}
