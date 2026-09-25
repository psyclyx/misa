use crate::prefs::DialogSettings;
use misa_lines::Line;
use misa_render::{Style, Theme};

/// The terminal's button/reference grammar. Actions remain semantic data; this is only the
/// client-side projection of their configured keys and labels.
pub(crate) fn reference<'a>(
    theme: &Theme,
    settings: &DialogSettings,
    actions: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<(Style, String)> {
    let mut spans = Vec::new();
    for (index, (id, label)) in actions.into_iter().enumerate() {
        if index > 0 {
            spans.push((theme.role("plain"), settings.action_separator.clone()));
        }
        if let Some(key) = settings.key(id) {
            spans.push((theme.role("keybinding"), display(key)));
            spans.push((theme.role("plain"), " ".into()));
        }
        spans.push((theme.role("label"), label.to_string()));
    }
    spans
}

/// Render configured key/label affordances that do not belong to dialog settings.
///
/// Pickers and selection readers have their own semantic action catalogues, but they use the
/// same reference grammar as dialogs. Keeping the grammar here prevents each surface from
/// inventing a slightly different "key label" string and leaves the source of the binding
/// (picker preferences, a dialog action, or a session declaration) outside the renderer.
pub(crate) fn key_reference<'a>(
    theme: &Theme,
    separator: &str,
    actions: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<(Style, String)> {
    let mut spans = Vec::new();
    for (index, (key, label)) in actions.into_iter().enumerate() {
        if index > 0 {
            spans.push((theme.role("plain"), separator.to_string()));
        }
        spans.push((theme.role("keybinding"), display(key)));
        spans.push((theme.role("plain"), " ".into()));
        spans.push((theme.role("label"), label.to_string()));
    }
    spans
}

pub fn footer<'a>(
    theme: &Theme,
    settings: &DialogSettings,
    actions: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Line {
    let mut spans = vec![(theme.role("dialog.label"), "└─ ".into())];
    spans.extend(reference(theme, settings, actions));
    Line {
        surface: None,
        indent: 0,
        node: None,
        spans,
    }
}

fn display(key: &str) -> String {
    key.split('+')
        .map(|part| match part {
            "alt" => "⌥",
            "ctrl" => "⌃",
            "shift" => "⇧",
            "up" => "↑",
            "down" => "↓",
            "left" => "←",
            "right" => "→",
            "enter" => "↵",
            "escape" => "esc",
            "tab" => "⇥",
            "pageup" => "pgup",
            "pagedown" => "pgdn",
            other => other,
        })
        .collect::<Vec<_>>()
        .concat()
}
