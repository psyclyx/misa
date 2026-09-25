//! Local document driver. Uses the same screen, retained layout, terminal intake
//! and frame emission as the connected loop, but has no session or request owner.
use crate::{Key, KeyOut, Screen, retained::Retained};
use crossterm::event::{self, Event, KeyEventKind};
use misa_proto::{Node, sync::ViewOp};
use std::io::Write;
use tokio::sync::mpsc;

pub enum Update {
    Reset(Node),
    Ops(Vec<ViewOp>),
}

/// Fixture-specific shortcuts are outside the screen's editing bindings. Return
/// an update to apply, `Consumed` for a local shortcut, or `Pass` to use Screen.
pub enum Control {
    Pass,
    Consumed,
    Update(Update),
    Quit,
}
pub trait Controller {
    fn key(&mut self, _: &mut Screen, _: &Node, _: &event::KeyEvent) -> Control {
        Control::Pass
    }
    /// The local document owner can answer actions offered by its own document.
    fn out(&mut self, _: &mut Screen, _: KeyOut) -> Control {
        Control::Pass
    }
}

pub async fn run<C: Controller>(
    screen: &mut Screen,
    initial: Node,
    controller: &mut C,
) -> Result<(), String> {
    let _terminal = crate::terminal_loop::Terminal::enter()?;
    let (_reader, events) = crate::terminal_loop::events();
    let (_sender, updates) = mpsc::channel(1);
    drive(
        screen,
        initial,
        controller,
        events,
        updates,
        &mut std::io::stdout(),
    )
    .await
}

/// Testable event loop. An update pipe permits local producers to change the
/// document even while no key is pressed; it carries protocol operations, not
/// a second renderer or a transport client.
pub async fn drive<C: Controller>(
    screen: &mut Screen,
    initial: Node,
    controller: &mut C,
    mut events: mpsc::Receiver<Result<Event, String>>,
    mut updates: mpsc::Receiver<Update>,
    writer: &mut impl Write,
) -> Result<(), String> {
    let mut retained = Retained::new(initial, screen);
    let mut output = misa_terminal_ui::output::Output::default();
    loop {
        let frame = retained.frame_with(screen, None, &[], &[]);
        screen.scroll = retained.resolved_scroll();
        screen.follow = retained.following();
        crate::terminal_loop::paint(writer, &mut output, screen, frame)?;
        let event = tokio::select! {
            biased;
            update = updates.recv(), if !updates.is_closed() => {
                if let Some(update) = update { apply(update, &mut retained, screen)?; }
                continue;
            }
            event = events.recv() => match event { Some(event) => event?, None => return Ok(()) },
        };
        match event {
            Event::Resize(width, height) => {
                screen.width = width;
                screen.height = height;
                output.invalidate();
            }
            Event::Paste(text) => {
                let view = retained.interaction();
                if crate::panel_of(&view).is_some() {
                    for character in text.chars() {
                        screen.panel_key(&view, &Key::Char(character));
                    }
                } else {
                    screen.paste(&text);
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
                event::MouseEventKind::ScrollUp => {
                    screen.key(Key::ScrollPage(-3));
                }
                event::MouseEventKind::ScrollDown => {
                    screen.key(Key::ScrollPage(3));
                }
                _ => continue,
            },
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let view = retained.interaction();
                match controller.key(screen, &view, &key) {
                    Control::Quit => return Ok(()),
                    Control::Consumed => {}
                    Control::Update(update) => apply(update, &mut retained, screen)?,
                    Control::Pass => {
                        let Some(key) =
                            crate::translate(key.code, key.modifiers, &screen.prefs.keymap)
                        else {
                            continue;
                        };
                        let out =
                            crate::terminal_loop::route_key(screen, &mut retained, &view, key);
                        match out {
                            KeyOut::Quit => return Ok(()),
                            KeyOut::StartSelection => {
                                retained.selection_key(screen, &Key::StartSelection);
                            }
                            KeyOut::Copy(text) => {
                                use base64::Engine as _;
                                let encoded =
                                    base64::engine::general_purpose::STANDARD.encode(text);
                                write!(writer, "\x1b]52;c;{encoded}\x07")
                                    .map_err(|e| e.to_string())?;
                                writer.flush().map_err(|e| e.to_string())?;
                            }
                            KeyOut::Local => {}
                            other => match controller.out(screen, other) {
                                Control::Quit => return Ok(()),
                                Control::Update(update) => apply(update, &mut retained, screen)?,
                                Control::Consumed => {}
                                Control::Pass => {
                                    screen.notice = Some(
                                        "Offline fixture: no session to receive this action".into(),
                                    )
                                }
                            },
                        }
                    }
                }
            }
            _ => continue,
        }
        retained.local(screen);
    }
}
fn apply(update: Update, retained: &mut Retained, screen: &Screen) -> Result<(), String> {
    match update {
        Update::Reset(node) => *retained = Retained::new(node, screen),
        Update::Ops(ops) => retained.apply_ops(&ops, screen)?,
    }
    Ok(())
}
