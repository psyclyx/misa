//! Semantic document rendering, independent of connections and HTTP lifetimes.
use misa_proto::view::{ActionOn, FieldKind, Kind, Node, Span, SpanKind, State as NodeState};

pub fn render_main(view: &Node) -> String {
    render_scoped(view, "")
}
pub(crate) fn render_scoped(view: &Node, prefix: &str) -> String {
    let mut out = String::new();
    render_node(view, prefix, &mut out);
    out
}

fn render_node(node: &Node, prefix: &str, out: &mut String) {
    let role = escape(&node.role);
    let id = if node.id.is_empty() {
        String::new()
    } else {
        format!(" id=\"{}{}\"", escape(prefix), escape(&node.id))
    };
    let state = node
        .state
        .map(|state| format!(" data-state=\"{}\"", state_word(state)))
        .unwrap_or_default();
    // A thematic break is a void element: it has no closing tag to match.
    if let Kind::Rule = &node.kind {
        out.push_str(&format!("<hr class=\"n-{role}\"{id}{state}>"));
        return;
    }

    // A node that offers a submit action is a form, and its fields are its inputs.
    // That is the only reason a view node ever becomes a form, and it is enough for
    // every dialog the shipped session has.
    let submit = node
        .actions
        .iter()
        .find(|action| action.on == ActionOn::Submit);
    if let Some(action) = submit {
        out.push_str(&format!(
            "<form class=\"n-{role}\"{id}{state} method=\"post\" action=\"./intent\">\
<input type=\"hidden\" name=\"node\" value=\"{node_id}\">\
<input type=\"hidden\" name=\"action\" value=\"{action}\">",
            role = role,
            id = id,
            state = state,
            node_id = escape(&node.id),
            action = escape(&action.id)
        ));
    } else {
        let element = element_for(node);
        out.push_str(&format!("<{element} class=\"n-{role}\"{id}{state}>"));
    }

    if let Some(label) = &node.label {
        out.push_str(&format!("<span class=\"label\">{}</span>", escape(label)));
    }

    match &node.kind {
        Kind::Section => {}
        Kind::Text { spans } => inline(spans, out),
        Kind::Heading { spans, .. } => inline(spans, out),
        // A quote's blocks and a rule's emptiness are structure, not content: the
        // element carries them and the stylesheet draws them. A rule is written before
        // this match, as a void element, so nothing goes inside it here.
        Kind::Quote => {}
        Kind::Rule => {}
        Kind::Code { lang, text } => {
            out.push_str("<pre><code");
            if let Some(lang) = lang {
                out.push_str(&format!(" data-lang=\"{}\"", escape(lang)));
            }
            out.push('>');
            // The browser highlights itself; this only carries the authored text and
            // the fence label the author wrote.
            out.push_str(&escape(text));
            out.push_str("</code></pre>");
        }
        Kind::List { items, markers, .. } => {
            // Each item is its own `<li>`; a nested list inside one is the child
            // nodes of that item, which the loop below emits. A task box replaces
            // the CSS marker, so the state survives to the browser.
            let mut items_out = String::new();
            for (index, item) in items.iter().enumerate() {
                match markers.get(index).copied().flatten() {
                    Some(checked) => {
                        items_out.push_str("<li class=\"task\">");
                        items_out.push_str(if checked {
                            "<span class=\"task-mark\">☑</span>"
                        } else {
                            "<span class=\"task-mark\">☐</span>"
                        });
                    }
                    None => items_out.push_str("<li>"),
                }
                for child in item {
                    render_node(child, prefix, &mut items_out);
                }
                items_out.push_str("</li>");
            }
            out.push_str(&items_out);
        }
        Kind::Table { head, rows, .. } => {
            out.push_str("<table>");
            if !head.is_empty() {
                out.push_str("<thead><tr>");
                for cell in head {
                    out.push_str("<th>");
                    inline(cell, out);
                    out.push_str("</th>");
                }
                out.push_str("</tr></thead>");
            }
            out.push_str("<tbody>");
            for row in rows {
                out.push_str("<tr>");
                for cell in row {
                    out.push_str("<td>");
                    inline(cell, out);
                    out.push_str("</td>");
                }
                out.push_str("</tr>");
            }
            out.push_str("</tbody></table>");
        }
        Kind::Fields { fields } => {
            out.push_str("<dl>");
            for field in fields {
                out.push_str(&format!("<dt>{}</dt><dd>", escape(&field.label)));
                match &field.kind {
                    _ if field.read_only => {
                        if field.secret {
                            out.push_str("••••");
                        } else if matches!(field.kind, FieldKind::Block) {
                            out.push_str(&format!("<pre>{}</pre>", escape(&field.value)));
                        } else {
                            out.push_str(&escape(&field.value));
                        }
                    },
                    _ if field.secret => out.push_str(&format!(
                        "<input type=\"password\" name=\"{}\" value=\"\">",
                        escape(&field.id)
                    )),
                    FieldKind::Block => out.push_str(&format!(
                        "<textarea name=\"{name}\" rows=\"3\" aria-label=\"{label}\" list=\"misa-commands\">{value}</textarea>",
                        name = escape(&field.id),
                        label = escape(&field.label),
                        value = escape(&field.value)
                    )),
                    FieldKind::Bool => out.push_str(&format!(
                        "<input type=\"checkbox\" name=\"{}\"{}>",
                        escape(&field.id),
                        if field.value == "true" { " checked" } else { "" }
                    )),
                    FieldKind::Choice { options, selected } => {
                        out.push_str(&format!("<select name=\"{}\">", escape(&field.id)));
                        for option in options {
                            out.push_str(&format!(
                                "<option value=\"{value}\"{selected}>{label}</option>",
                                value = escape(&option.value),
                                label = escape(&option.label),
                                selected = if Some(&option.value) == selected.as_ref() { " selected" } else { "" }
                            ));
                        }
                        out.push_str("</select>");
                    }
                    FieldKind::Inline => out.push_str(&format!(
                        "<input type=\"text\" name=\"{}\" value=\"{}\">",
                        escape(&field.id),
                        escape(&field.value)
                    )),
                }
                if let Some(hint) = &field.hint {
                    out.push_str(&format!("<small>{}</small>", escape(hint)));
                }
                out.push_str("</dd>");
            }
            out.push_str("</dl>");
        }
        Kind::Collapsible { summary } => {
            // `<details>` is why this node exists as a node: a short form and a long
            // form are a thing HTML already has a word for.
            out.push_str("<summary>");
            inline(summary, out);
            out.push_str("</summary>");
        }
        Kind::Image { blob, alt, .. } => {
            // The browser renders the shared image node and keeps its alternative text.
            out.push_str(&format!(
                "<img src=\"./blob/{hash}\" alt=\"{alt}\">",
                hash = escape(&blob.hash),
                alt = escape(alt)
            ));
        }
        Kind::Status { text } => out.push_str(&escape(text)),
        // A fact is written by the client's own formatter, and marked up so a
        // stylesheet can treat a number differently from a word. `<data>` says
        // exactly that: here is a value, and here is how to write it.
        Kind::Fact { value } => out.push_str(&format!(
            "<data value=\"{}\">{}</data>",
            escape(&value.to_string()),
            escape(&misa_render::fact::format(&node.role, value))
        )),
        Kind::Meter { label, value, max } => {
            out.push_str(&format!(
                "<meter min=\"0\" max=\"{max}\" value=\"{value}\" aria-label=\"{label}\"></meter>\
<span class=\"value\">{text}</span>",
                max = max,
                value = value,
                label = escape(label),
                text = format!("{value}/{max}")
            ));
        }
    }

    for child in &node.children {
        render_node(child, prefix, out);
    }

    // A node that is not a form may still offer a click action, and a button that is not in a
    // form posts nothing: each one gets a one-button form of its own, which is the whole of
    // what it takes for a panel's buttons to work in a browser with no script at all.
    if submit.is_none() {
        for action in &node.actions {
            if action.id == "attachment.save" {
                out.push_str(&format!("<form method=\"post\" action=\"./download\"><input type=\"hidden\" name=\"node\" value=\"{}\"><button type=\"submit\">{}</button></form>", escape(&node.id), escape(action.label.as_deref().unwrap_or("Save attachment"))));
                continue;
            }
            out.push_str(&format!(
                "<form class=\"n-{role}.action\" method=\"post\" action=\"./intent\">\
<input type=\"hidden\" name=\"node\" value=\"{node_id}\">\
<input type=\"hidden\" name=\"action\" value=\"{action}\">\
<button type=\"submit\">{label}</button></form>",
                role = role,
                node_id = escape(&node.id),
                action = escape(&action.id),
                label = escape(action.label.as_deref().unwrap_or(&action.id))
            ));
        }
    }
    if submit.is_some() {
        out.push_str("<button type=\"submit\">");
        out.push_str(&escape(
            submit
                .and_then(|action| action.label.as_deref())
                .unwrap_or("Send"),
        ));
        out.push_str("</button></form>");
    } else {
        let element = element_for(node);
        out.push_str(&format!("</{element}>"));
    }
}

fn element_for(node: &Node) -> String {
    match &node.kind {
        Kind::Text { .. } => "p".into(),
        // HTML has one element per heading level, so the level picks the tag.
        Kind::Heading { level, .. } => format!("h{}", (*level).clamp(1, 6)),
        Kind::Code { .. } => "div".into(),
        Kind::List { ordered: true, .. } => "ol".into(),
        Kind::List { .. } => "ul".into(),
        Kind::Table { .. } => "div".into(),
        Kind::Fields { .. } => "dl".into(),
        Kind::Collapsible { .. } => "details".into(),
        Kind::Image { .. } => "figure".into(),
        Kind::Status { .. } => "p".into(),
        Kind::Fact { .. } => "data".into(),
        Kind::Meter { .. } => "p".into(),
        Kind::Quote => "blockquote".into(),
        // A rule is written before this is reached; named here so the match stays total.
        Kind::Rule => "hr".into(),
        Kind::Section => "section".into(),
    }
}

fn inline(spans: &[Span], out: &mut String) {
    for span in spans {
        let (open, close) = span_tags(&span.kind);
        if span.text.contains('\n') {
            // A newline inside a run is a paragraph the session chose; the browser
            // is told with elements rather than with `white-space`.
            let mut first = true;
            for part in span.text.split('\n') {
                if !first {
                    out.push_str(&format!("</{close}>"));
                    out.push_str(&format!("<{open}>"));
                }
                first = false;
                out.push_str(&escape(part));
            }
            continue;
        }
        out.push_str(&format!("<{open}>"));
        out.push_str(&escape(&span.text));
        out.push_str(&format!("</{close}>"));
    }
}

/// The opening tag content and the closing tag name for a span.
///
/// Returned separately because an attribute-bearing tag (`a href=…`) closes as
/// just `a`, and a combined mark nests.
fn span_tags(kind: &SpanKind) -> (String, String) {
    match kind {
        SpanKind::Plain => ("span".into(), "span".into()),
        SpanKind::Strong => ("strong".into(), "strong".into()),
        SpanKind::StrongEmphasis => ("strong><em".into(), "em></strong".into()),
        SpanKind::Emphasis => ("em".into(), "em".into()),
        SpanKind::Strikethrough => ("del".into(), "del".into()),
        SpanKind::Underline => ("u".into(), "u".into()),
        SpanKind::Highlight => ("mark".into(), "mark".into()),
        SpanKind::Subscript => ("sub".into(), "sub".into()),
        SpanKind::Superscript => ("sup".into(), "sup".into()),
        SpanKind::Kbd => ("kbd".into(), "kbd".into()),
        SpanKind::Code => ("code".into(), "code".into()),
        SpanKind::Link { href } => (format!("a href=\"{}\"", escape(href)), "a".into()),
    }
}

fn state_word(state: NodeState) -> &'static str {
    match state {
        NodeState::Pending => "pending",
        NodeState::Streaming => "streaming",
        NodeState::Done => "done",
        NodeState::Failed => "failed",
        NodeState::Cancelled => "cancelled",
    }
}

/// HTML-escape everything a session sends. A view tree is data and may contain
/// anything a model wrote, so nothing reaches a browser unescaped.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(character),
        }
    }
    out
}
