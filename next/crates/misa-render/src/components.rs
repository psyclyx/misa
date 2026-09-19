//! Client component implementations and their independently configurable selection.
//! The semantic tree supplies facts; no component knows the agent's database.
use crate::{Line, Theme};
use misa_proto::view::{Kind, Node};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    #[default]
    Document,
    Footer,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Representation {
    #[default]
    LabelValue,
    Value,
    Icon,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub id: String,
    #[serde(default)]
    pub representation: Representation,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub hint: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub roles: BTreeMap<String, String>,
    pub placement: BTreeMap<String, Placement>,
    /// None means the shipped selection; Some([]) explicitly hides all indicators.
    pub indicators: Option<Vec<Selection>>,
}

pub struct Context<'a> {
    pub theme: &'a Theme,
    pub columns: usize,
    pub settings: &'a Settings,
    pub values: &'a crate::fact::Registry,
}
pub type Render = Arc<dyn Fn(&Node, &Context<'_>) -> Result<Vec<Line>, String> + Send + Sync>;
#[derive(Clone)]
pub struct Component {
    pub render: Render,
    pub placement: Placement,
}
#[derive(Clone)]
pub struct Registry {
    implementations: BTreeMap<String, Component>,
    defaults: BTreeMap<String, String>,
}

impl Registry {
    pub fn empty() -> Self {
        Self {
            implementations: BTreeMap::new(),
            defaults: BTreeMap::new(),
        }
    }
    pub fn register(&mut self, id: &str, component: Component) -> Result<(), String> {
        if id.is_empty() || self.implementations.contains_key(id) {
            return Err(format!("Duplicate or empty component `{id}`"));
        }
        self.implementations.insert(id.into(), component);
        Ok(())
    }
    pub fn select_default(&mut self, role: &str, implementation: &str) -> Result<(), String> {
        if !self.implementations.contains_key(implementation) {
            return Err(format!("Unknown component `{implementation}`"));
        }
        self.defaults.insert(role.into(), implementation.into());
        Ok(())
    }
    fn resolve(&self, role: &str, settings: &Settings) -> Option<&Component> {
        let id = settings
            .roles
            .get(role)
            .or_else(|| self.defaults.get(role))?;
        self.implementations.get(id)
    }
    pub fn placement(&self, role: &str, settings: &Settings) -> Placement {
        settings.placement.get(role).copied().unwrap_or_else(|| {
            self.resolve(role, settings)
                .map(|c| c.placement)
                .unwrap_or_default()
        })
    }
    pub fn render(&self, node: &Node, context: &Context<'_>) -> Option<Vec<Line>> {
        let component = self.resolve(&node.role, context.settings)?;
        Some((component.render)(node, context).unwrap_or_else(|reason| {
            vec![Line {
                node: Some(node.id.clone()),
                indent: 0,
                surface: None,
                spans: vec![(
                    context.theme.role("error"),
                    crate::clip(&format!("{}: {reason}", node.role), context.columns),
                )],
            }]
        }))
    }
}

impl Default for Registry {
    fn default() -> Self {
        let mut registry = Self::empty();
        registry
            .register(
                "default.status.indicators",
                Component {
                    render: Arc::new(indicators),
                    placement: Placement::Footer,
                },
            )
            .unwrap();
        registry
            .select_default("status.indicators", "default.status.indicators")
            .unwrap();
        registry
            .register(
                "default.message.group.footer",
                Component {
                    render: Arc::new(group_footer),
                    placement: Placement::Document,
                },
            )
            .unwrap();
        registry
            .register(
                "default.queue",
                Component {
                    render: Arc::new(queue),
                    placement: Placement::Footer,
                },
            )
            .unwrap();
        registry
            .select_default("message.group.footer", "default.message.group.footer")
            .unwrap();
        registry.select_default("queue", "default.queue").unwrap();
        registry
    }
}

pub fn stock() -> &'static Registry {
    static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(Registry::default)
}

pub fn render_default(node: &Node, theme: &Theme, columns: usize) -> Option<Vec<Line>> {
    stock().render(
        node,
        &Context {
            theme,
            columns,
            settings: &Settings::default(),
            values: &crate::fact::Registry::default(),
        },
    )
}

/// Defaults are registration/configuration data, never branches in the renderer.
pub fn default_indicators() -> Vec<Selection> {
    // This is the reference configuration's shipped selection, in its actual
    // order, which places `cost` second. Turns remains an available fact, but is not
    // selected by default.
    [
        ("activity", 100, Representation::Icon, Some("●"), None),
        ("cost", 85, Representation::LabelValue, None, None),
        ("model", 90, Representation::Value, None, Some("alt+m")),
        (
            "effort",
            70,
            Representation::LabelValue,
            None,
            Some("alt+f"),
        ),
        ("session", 40, Representation::Icon, Some("tok"), None),
        ("context", 80, Representation::LabelValue, None, None),
        ("plan", 60, Representation::LabelValue, None, None),
        (
            "transcript-detail",
            20,
            Representation::LabelValue,
            None,
            Some("alt+t"),
        ),
    ]
    .into_iter()
    .map(|(id, priority, representation, icon, hint)| Selection {
        id: id.into(),
        priority,
        representation,
        icon: icon.map(str::to_string),
        hint: hint.map(str::to_string),
    })
    .collect()
}

/// Preserve configured order, dropping complete low-priority indicators to fit.
pub fn selected<'a>(
    model: &'a Node,
    settings: &Settings,
    columns: usize,
    values: &crate::fact::Registry,
) -> Vec<(&'a Node, Selection, String)> {
    let defaults = default_indicators();
    let selection = settings.indicators.as_ref().unwrap_or(&defaults);
    let mut items: Vec<_> = selection
        .iter()
        .filter_map(|selection| {
            let node = model
                .children
                .iter()
                .find(|node| node.role.strip_prefix("indicator.") == Some(&selection.id))?;
            let text = indicator_text(node, selection, values);
            Some((node, selection.clone(), text))
        })
        .collect();
    while items
        .iter()
        .map(|(_, _, text)| crate::width(text))
        .sum::<usize>()
        + items.len().saturating_sub(1) * 2
        > columns
    {
        let Some(victim) = items
            .iter()
            .enumerate()
            .min_by_key(|(index, (_, selection, _))| {
                (selection.priority, std::cmp::Reverse(*index))
            })
            .map(|(index, _)| index)
        else {
            break;
        };
        items.remove(victim);
    }
    items
}

fn indicator_text(node: &Node, selection: &Selection, values: &crate::fact::Registry) -> String {
    indicator_spans(node, selection, values, &Theme::plain())
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

fn indicator_value(node: &Node, values: &crate::fact::Registry) -> String {
    match &node.kind {
        Kind::Fact { value } => values.format(&node.role, value),
        Kind::Meter { value, max, .. } => format!(
            "{}/{}",
            crate::fact::count(Some(*value as i64), ""),
            crate::fact::count(Some(*max as i64), "")
        ),
        Kind::Status { text } => text.clone(),
        Kind::Text { spans } => spans.iter().map(|span| span.text.as_str()).collect(),
        _ => node
            .children
            .iter()
            .map(|node| indicator_value(node, values))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn key_hint(hint: &str) -> String {
    hint.split('+')
        .map(|part| match part {
            "alt" => "⌥",
            "ctrl" => "⌃",
            "shift" => "⇧",
            "enter" => "↵",
            "escape" => "esc",
            "tab" => "⇥",
            other => other,
        })
        .collect::<Vec<_>>()
        .concat()
}

/// Keep the status bar's label, value and key hint as distinct semantic runs.
/// The old client used those roles to make the bar readable at a glance; one
/// preformatted string loses that contract and makes themes unable to tune it.
fn indicator_spans(
    node: &Node,
    selection: &Selection,
    values: &crate::fact::Registry,
    theme: &Theme,
) -> Vec<(crate::Style, String)> {
    let value = indicator_value(node, values);
    let label = match selection.representation {
        Representation::Value => None,
        Representation::Icon => selection
            .icon
            .as_deref()
            .or(node.label.as_deref())
            .filter(|label| !label.is_empty()),
        Representation::LabelValue => node.label.as_deref().filter(|label| !label.is_empty()),
    };
    let mut spans = Vec::new();
    if let Some(label) = label {
        spans.push((theme.role("label"), label.to_string()));
        spans.push((theme.role("plain"), " ".into()));
    }
    spans.push((theme.role("value"), value));
    if let Some(hint) = &selection.hint {
        spans.push((theme.role("plain"), " ".into()));
        spans.push((theme.role("keybinding"), key_hint(hint)));
    }
    spans
}

fn indicators(node: &Node, context: &Context<'_>) -> Result<Vec<Line>, String> {
    let mut line = Line {
        node: Some(node.id.clone()),
        ..Line::default()
    };
    for (node, selection, _) in selected(node, context.settings, context.columns, context.values) {
        if !line.spans.is_empty() {
            line.spans
                .push((context.theme.role("status.separator"), "  ".into()));
        }
        line.spans.extend(indicator_spans(
            node,
            &selection,
            context.values,
            context.theme,
        ));
    }
    Ok(if line.spans.is_empty() {
        vec![]
    } else {
        vec![line]
    })
}

/// The transcript boundary is a component because its facts are shared while its
/// line geometry is not. It deliberately consumes typed child nodes rather than
/// making the session assemble a preformatted footer string.
fn group_footer(node: &Node, context: &Context<'_>) -> Result<Vec<Line>, String> {
    let mut text = String::from("──");
    for child in &node.children {
        let value = match &child.kind {
            Kind::Fact { value } => context.values.format(&child.role, value),
            Kind::Text { spans } => spans.iter().map(|span| span.text.as_str()).collect(),
            _ => String::new(),
        };
        if !value.is_empty() {
            text.push_str("  ");
            text.push_str(&value);
        }
    }
    let width = context.columns.max(1);
    let used = crate::width(&text);
    if used < width {
        text.push(' ');
        text.push_str(&"─".repeat(width.saturating_sub(used + 1)));
    }
    Ok(vec![Line {
        node: Some(node.id.clone()),
        indent: 0,
        surface: None,
        spans: vec![(
            context.theme.role("message.group.footer"),
            crate::clip(&text, width),
        )],
    }])
}

/// Pending prompts are an input dock, not transcript content. Keeping this as a
/// footer component gives the session a semantic queue while placing it beside
/// the composer on a terminal and in the equivalent dock on other clients.
fn queue(node: &Node, context: &Context<'_>) -> Result<Vec<Line>, String> {
    let count = node
        .children
        .iter()
        .find(|child| child.role == "queue.count")
        .and_then(|child| match &child.kind {
            Kind::Fact { value } => value.as_i64(),
            _ => None,
        })
        .unwrap_or_else(|| {
            node.children
                .iter()
                .filter(|child| child.role == "queue.item")
                .count() as i64
        });
    // The queue's semantic actions have short terminal labels in the reference
    // dock. Their long labels remain on the shared action surface; this is only
    // the compact rendering of the same action ids.
    let actions = node
        .actions
        .iter()
        .map(|action| match action.id.as_str() {
            "queue.edit" => "edit",
            "queue.steer" => "send now",
            _ => action.label.as_deref().unwrap_or(action.id.as_str()),
        })
        .collect::<Vec<_>>();
    let heading = if actions.is_empty() {
        format!("Queued ({count})")
    } else {
        format!("Queued  {}", actions.join(" · "))
    };
    let mut lines = vec![Line {
        node: Some(node.id.clone()),
        indent: 0,
        surface: None,
        spans: vec![(context.theme.role("label"), heading)],
    }];
    for child in node
        .children
        .iter()
        .filter(|child| child.role == "queue.item")
    {
        if let Kind::Text { spans } = &child.kind {
            let text: String = spans.iter().map(|span| span.text.as_str()).collect();
            lines.push(Line {
                node: Some(child.id.clone()),
                indent: 0,
                surface: None,
                spans: vec![(
                    context.theme.role("dim"),
                    crate::clip(&text.replace('\n', " ↵ "), context.columns),
                )],
            });
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_value::Value;
    fn item(id: &str, value: i64) -> Node {
        Node::section(format!("indicator.{id}"))
            .id(id)
            .label(id)
            .child(Node::new(
                "value.watts",
                Kind::Fact {
                    value: Value::Int(value),
                },
            ))
    }
    #[test]
    fn contributed_values_selection_and_priority_need_no_widget_branches() {
        let model = Node::section("status.indicators").id("status").children([
            item("battery", 7),
            item("solar", 12),
            item("spare", 2),
        ]);
        let mut values = crate::fact::Registry::default();
        values
            .register(
                "value.watts",
                Arc::new(|value| format!("{}W", value.as_i64().unwrap())),
            )
            .unwrap();
        let settings = Settings {
            indicators: Some(vec![
                Selection {
                    id: "solar".into(),
                    priority: 100,
                    representation: Representation::Value,
                    icon: None,
                    hint: None,
                },
                Selection {
                    id: "battery".into(),
                    priority: 20,
                    representation: Representation::LabelValue,
                    icon: None,
                    hint: None,
                },
            ]),
            ..Settings::default()
        };
        let selected = selected(&model, &settings, 80, &values);
        assert_eq!(
            selected
                .iter()
                .map(|(node, _, text)| (node.id.as_str(), text.as_str()))
                .collect::<Vec<_>>(),
            [("solar", "12W"), ("battery", "battery 7W")]
        );
        let narrow = super::selected(&model, &settings, 8, &values);
        assert_eq!(narrow.len(), 1);
        assert_eq!(narrow[0].0.id, "solar");
        assert!(super::selected(&model, &settings, 2, &values).is_empty());
        assert!(
            super::selected(
                &model,
                &Settings {
                    indicators: Some(vec![]),
                    ..Settings::default()
                },
                80,
                &values
            )
            .is_empty()
        );
    }
    #[test]
    fn component_implementation_and_placement_are_selected_independently() {
        let mut registry = Registry::default();
        registry
            .register(
                "custom.summary",
                Component {
                    placement: Placement::Document,
                    render: Arc::new(|node, context| {
                        Ok(vec![Line {
                            node: Some(node.id.clone()),
                            indent: 0,
                            surface: None,
                            spans: vec![(context.theme.role("notice"), "custom component".into())],
                        }])
                    }),
                },
            )
            .unwrap();
        let mut settings = Settings::default();
        settings
            .roles
            .insert("status.indicators".into(), "custom.summary".into());
        settings
            .placement
            .insert("status.indicators".into(), Placement::Footer);
        assert_eq!(
            registry.placement("status.indicators", &settings),
            Placement::Footer
        );
        let lines = registry
            .render(
                &Node::section("status.indicators"),
                &Context {
                    settings: &settings,
                    values: &crate::fact::Registry::default(),
                    theme: &Theme::plain(),
                    columns: 80,
                },
            )
            .unwrap();
        assert_eq!(lines[0].text(), "custom component");
    }

    #[test]
    fn shipped_indicator_selection_matches_the_reference_order_and_membership() {
        assert_eq!(
            default_indicators()
                .into_iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [
                "activity",
                "cost",
                "model",
                "effort",
                "session",
                "context",
                "plan",
                "transcript-detail"
            ],
        );
    }
}
