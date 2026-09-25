//! Standalone, local-only terminal fixture runner. No Workspace or session is constructed.
//! Run with: cargo run -p misa-tui --no-default-features --bin misa-tui-testbed
//! n/Right/Tab and p/Left cycle scenes; j/k scroll; t changes theme; q/Esc quits.
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use misa_lines::Line;
use misa_proto::view::{Kind, Node, Span, State};
use misa_render::Theme;
use std::io::{self, IsTerminal, Write};
use std::time::Duration;

// Reuse the actual terminal painter (and its ANSI encoder), not a mock screenshot.
// These modules do not import the production screen or its transport dependencies.
pub use misa_linear::graphics;
#[path = "../terminal_style.rs"]
mod terminal_style;
pub use terminal_style::sgr;
#[path = "../output.rs"]
mod output;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scene {
    Native,
    Semantic,
    Structured,
}
impl Scene {
    const ALL: [Self; 3] = [Self::Native, Self::Semantic, Self::Structured];
    fn label(self) -> &'static str {
        match self {
            Self::Native => "native component / lines",
            Self::Semantic => "semantic transcript",
            Self::Structured => "semantic structures",
        }
    }
}

/// The first scene constructs lines and invokes the stock component directly;
/// the others go through the misa-proto -> misa-lines rendering boundary.
fn fixture(scene: Scene, theme: &Theme, width: usize) -> Vec<Line> {
    let width = width.max(1);
    match scene {
        Scene::Native => {
            let mut lines = vec![Line {
                node: Some("native.title".into()),
                spans: vec![(
                    theme.role("header.title"),
                    misa_render::clip("Direct Line fixture", width),
                )],
                ..Line::default()
            }];
            let queue = Node::section("queue").id("native.queue").child(
                Node::text("queue.item", [Span::plain("An item in the queue")])
                    .id("native.queue.item"),
            );
            let context = misa_lines::components::Context {
                theme,
                columns: width,
                settings: &Default::default(),
                values: misa_render::fact::stock(),
            };
            lines.extend(
                misa_lines::components::stock()
                    .render(&queue, &context)
                    .expect("stock queue component"),
            );
            lines.push(Line {
                node: Some("native.footer".into()),
                spans: vec![(
                    theme.role("notice"),
                    misa_render::clip(
                        "Lines + stock queue component, no semantic document renderer",
                        width,
                    ),
                )],
                ..Line::default()
            });
            lines
        }
        Scene::Semantic => misa_lines::render(&semantic_transcript(), theme, width),
        Scene::Structured => misa_lines::render(&semantic_structures(), theme, width),
    }
}
fn semantic_transcript() -> Node {
    Node::section("session")
        .id("fixture.session")
        .child(
            Node::text("session.title", [Span::plain("Fixture conversation")]).id("fixture.title"),
        )
        .child(
            Node::section("transcript")
                .id("fixture.transcript")
                .child(
                    Node::text(
                        "message.user",
                        [Span::plain("How does a local fixture render?")],
                    )
                    .id("fixture.user"),
                )
                .child(
                    Node::text(
                        "message.assistant",
                        [Span::plain(
                            "Through the semantic line renderer and the real terminal painter.",
                        )],
                    )
                    .id("fixture.answer")
                    .state(State::Done),
                )
                .child(
                    Node::new(
                        "tool.result",
                        Kind::Code {
                            lang: Some("rust".into()),
                            text: "let connected = false;\nassert!(!connected);".into(),
                        },
                    )
                    .id("fixture.code"),
                ),
        )
}
fn semantic_structures() -> Node {
    Node::section("session")
        .id("fixture.structures")
        .child(
            Node::new(
                "heading",
                Kind::Heading {
                    level: 2,
                    spans: vec![Span::plain("Semantic structures")],
                },
            )
            .id("fixture.heading"),
        )
        .child(
            Node::new(
                "list",
                Kind::List {
                    ordered: false,
                    items: vec![
                        vec![
                            Node::text("text", [Span::plain("First nested item")])
                                .id("fixture.one"),
                        ],
                        vec![
                            Node::text("text", [Span::plain("Second nested item")])
                                .id("fixture.two"),
                        ],
                    ],
                    markers: vec![Some(true), Some(false)],
                },
            )
            .id("fixture.list"),
        )
        .child(
            Node::new(
                "status",
                Kind::Meter {
                    label: "Local progress".into(),
                    value: 2.0,
                    max: 3.0,
                },
            )
            .id("fixture.meter"),
        )
}

struct Testbed {
    scene: usize,
    theme: usize,
    scroll: usize,
    width: u16,
    height: u16,
    painter: output::Output,
}
impl Testbed {
    fn new(width: u16, height: u16) -> Self {
        Self {
            scene: 0,
            theme: 0,
            scroll: 0,
            width,
            height,
            painter: output::Output::default(),
        }
    }
    fn theme(&self) -> Theme {
        match self.theme {
            1 => Theme::light(),
            2 => Theme::plain(),
            _ => Theme::dark(),
        }
    }
    fn rows(&self) -> Vec<Line> {
        let theme = self.theme();
        let width = self.width.max(1) as usize;
        let available = (self.height as usize).saturating_sub(2);
        let content = fixture(Scene::ALL[self.scene], &theme, width);
        let max_scroll = content.len().saturating_sub(available);
        let mut rows: Vec<_> = content
            .into_iter()
            .skip(self.scroll.min(max_scroll))
            .take(available)
            .collect();
        let style = theme.role("status");
        let header = format!(
            "Fixture {}/{} · {} · theme {}",
            self.scene + 1,
            Scene::ALL.len(),
            Scene::ALL[self.scene].label(),
            ["dark", "light", "plain"][self.theme]
        );
        let help = "n/p or ←/→: scene  j/k: scroll  t: theme  q/Esc: quit";
        if self.height >= 2 {
            while rows.len() < available {
                rows.push(Line::default());
            }
            rows.push(Line {
                spans: vec![(style, misa_render::clip(&header, width))],
                ..Line::default()
            });
            rows.push(Line {
                spans: vec![(style, misa_render::clip(help, width))],
                ..Line::default()
            });
        }
        rows
    }
    fn key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return false,
            KeyCode::Char('n') | KeyCode::Right | KeyCode::Tab => {
                self.scene = (self.scene + 1) % Scene::ALL.len();
                self.scroll = 0;
            }
            KeyCode::Char('p') | KeyCode::Left | KeyCode::BackTab => {
                self.scene = (self.scene + Scene::ALL.len() - 1) % Scene::ALL.len();
                self.scroll = 0;
            }
            KeyCode::Char('t') => self.theme = (self.theme + 1) % 3,
            KeyCode::Char('j') | KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            _ => {}
        }
        true
    }
    fn paint(&mut self, writer: &mut impl Write) -> io::Result<()> {
        self.painter
            .paint(writer, &self.rows(), self.width as usize, &[])?;
        // Place the cursor on the status row, not inside the fixture.
        write!(writer, "\x1b[{};1H", self.height.max(1))?;
        writer.flush()
    }
}

struct Terminal;
impl Terminal {
    fn enter() -> io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        if let Err(error) =
            crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen)
        {
            crossterm::terminal::disable_raw_mode()?;
            return Err(error);
        }
        Ok(Self)
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen);
        let _ = crossterm::terminal::disable_raw_mode();
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "misa-tui-testbed: local fixtures only; n/p cycle, j/k scroll, t theme, q/Esc quit"
        );
        return Ok(());
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("testbed requires a terminal (try --help)".into());
    }
    let (width, height) = crossterm::terminal::size()?;
    let _terminal = Terminal::enter()?;
    let mut testbed = Testbed::new(width, height);
    let mut stdout = io::stdout();
    testbed.paint(&mut stdout)?;
    loop {
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if !testbed.key(key.code) {
                    break;
                }
            }
            Event::Resize(width, height) => {
                testbed.width = width;
                testbed.height = height;
                testbed.painter.invalidate();
            }
            _ => continue,
        }
        testbed.paint(&mut stdout)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixtures_render_through_real_lines_and_component() {
        let theme = Theme::dark();
        let native = fixture(Scene::Native, &theme, 60);
        assert!(native.iter().any(|line| line.text().contains("Queued (1)")));
        assert!(
            native
                .iter()
                .any(|line| line.node.as_deref() == Some("native.queue.item"))
        );
        for scene in [Scene::Semantic, Scene::Structured] {
            let lines = fixture(scene, &theme, 60);
            assert!(lines.len() > 1, "{scene:?}");
            assert!(lines.iter().any(|line| line.node.is_some()));
        }
    }
    #[test]
    fn cycles_and_repaints_actual_output_without_a_session() {
        let mut testbed = Testbed::new(60, 12);
        let mut bytes = Vec::new();
        testbed.paint(&mut bytes).unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("Direct Line fixture"));
        assert!(testbed.key(KeyCode::Char('n')));
        bytes.clear();
        testbed.paint(&mut bytes).unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("Fixture conversation"));
        testbed.key(KeyCode::Char('p'));
        assert_eq!(testbed.scene, 0);
        testbed.key(KeyCode::Char('t'));
        assert_eq!(testbed.theme, 1);
        assert!(!testbed.key(KeyCode::Esc));
    }
    #[test]
    fn tiny_terminal_and_resize_do_not_panic() {
        let mut testbed = Testbed::new(0, 0);
        testbed.paint(&mut Vec::new()).unwrap();
        testbed.width = 8;
        testbed.height = 2;
        testbed.painter.invalidate();
        testbed.paint(&mut Vec::new()).unwrap();
        assert_eq!(testbed.rows().len(), 2);
    }
}
