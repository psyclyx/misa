//! Keyboard and local animation remain live while the session has nothing to send.
use std::io::Write;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use crossterm::event::{self, Event, KeyEventKind};
use tokio::sync::mpsc;
use crate::{KeyOut, Screen, Session};

struct Terminal;
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), event::DisableBracketedPaste,
            crossterm::terminal::LeaveAlternateScreen);
        let _ = crossterm::terminal::disable_raw_mode();
    }
}
struct Reader { stop: Arc<AtomicBool>, thread: Option<std::thread::JoinHandle<()>> }
impl Drop for Reader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

pub async fn run(session: &mut dyn Session) -> Result<(), String> {
    let mut screen = Screen::durable();
    if let Some(info) = session.info() { screen.declare(&info); }
    if let Ok((width, height)) = crossterm::terminal::size() { screen.width = width; screen.height = height; }
    crossterm::terminal::enable_raw_mode().map_err(|error| error.to_string())?;
    let _terminal = Terminal;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen,
        event::EnableBracketedPaste).map_err(|error| error.to_string())?;
    let (sender, receiver) = mpsc::unbounded_channel();
    let stop = Arc::new(AtomicBool::new(false));
    let reading = stop.clone();
    let thread = std::thread::spawn(move || {
        while !reading.load(Ordering::Relaxed) {
            match event::poll(Duration::from_millis(50)) {
                Ok(false) => continue,
                Ok(true) => if sender.send(event::read().map_err(|error| error.to_string())).is_err() { break; },
                Err(error) => { let _ = sender.send(Err(error.to_string())); break; }
            }
        }
    });
    let _reader = Reader { stop, thread: Some(thread) };
    let result = drive(session, &mut screen, receiver, &mut std::io::stdout(), &mut crate::clipboard::Desktop).await;
    screen.save();
    result
}

async fn drive(session: &mut dyn Session, screen: &mut Screen,
    mut events: mpsc::UnboundedReceiver<Result<Event, String>>, writer: &mut impl Write, clipboard: &mut dyn crate::clipboard::Source) -> Result<(), String> {
    let Some(mut view) = session.next().await? else { return Ok(()); };
    let mut pending = Vec::<misa_proto::view::BlobRef>::new();
    let mut output = crate::output::Output::default();
    let mut animation = tokio::time::interval(Duration::from_millis(90));
    animation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frame = 0usize;
    loop {
        let mut lines = crate::draw(screen, &view);
        if !pending.is_empty() {
            let line=misa_render::Line {node:None,indent:0,spans:vec![(screen.theme.role("notice"),format!("{} clipboard attachments · Ctrl-Alt-V discards",pending.len()))]};
            lines.insert(lines.len().saturating_sub(1),line);
        }
        if misa_proto::view::find(&view, "turn").is_some() {
            const FRAMES: [&str; 4] = ["⠋", "⠙", "⠹", "⠸"];
            if let Some(line) = lines.iter_mut().find(|line| line.node.as_deref() == Some("turn")) {
                line.spans.insert(0, (screen.theme.role("turn"), format!("{} ", FRAMES[frame % FRAMES.len()])));
            }
        }
        output.paint(writer, &lines).map_err(|error| error.to_string())?;
        let (before, _) = screen.editor.split_at_cursor();
        let column = (2 + misa_render::width(before.rsplit('\n').next().unwrap_or("")))
            .min(screen.width.saturating_sub(1) as usize);
        write!(writer, "\x1b[{};{}H", lines.len().max(1), column + 1)
            .and_then(|_| writer.flush()).map_err(|error| error.to_string())?;
        let event = tokio::select! {
            incoming = session.next() => {
                match incoming? { Some(next) => view = next, None => return Ok(()) }
                continue;
            }
            _ = animation.tick(), if misa_proto::view::find(&view, "turn").is_some() => {
                frame = frame.wrapping_add(1); continue;
            }
            event = events.recv() => match event { Some(event) => event?, None => return Ok(()) },
        };
        let out = match event {
            Event::Resize(width, height) => { screen.width = width; screen.height = height; output.invalidate(); continue; }
            Event::Paste(text) => {
                if crate::panel_of(&view).is_some() {
                    for character in text.chars() { screen.panel_key(&view, &crate::Key::Char(character)); }
                } else { screen.editor.insert(&text); }
                continue;
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == event::KeyCode::Char('v') && key.modifiers.contains(event::KeyModifiers::CONTROL) {
                    if key.modifiers.contains(event::KeyModifiers::ALT) { pending.clear();screen.notice=Some("Clipboard attachments discarded".into());continue; }
                    match clipboard.read() {
                        Ok(crate::clipboard::Contents::Text(text)) => {
                            if crate::panel_of(&view).is_some() {for character in text.chars() {screen.panel_key(&view,&crate::Key::Char(character));}}
                            else {screen.editor.insert(&text);}
                        },
                        Ok(crate::clipboard::Contents::Image {width,height,rgba}) => {
                            if crate::panel_of(&view).is_some() {screen.notice=Some("Close the panel to attach an image to the prompt".into());continue;}
                            let result=async {session.upload(crate::clipboard::png(width,height,rgba)?,"image/png").await}.await;
                            match result {Ok(blob)=>{pending.push(blob);screen.notice=Some("Image attached; Enter sends the prompt".into());},Err(error)=>screen.notice=Some(error)}
                        },
                        Err(error)=>screen.notice=Some(error),
                    }
                    continue;
                }
                if key.code == event::KeyCode::Enter && key.modifiers.is_empty() && screen.editor.text().is_empty() && !pending.is_empty() && crate::panel_of(&view).is_none() {
                    KeyOut::Intent(misa_proto::Intent::Prompt {text:String::new(),attachments:vec![]})
                } else {
                let Some(key) = crate::translate(key.code, key.modifiers) else { continue; };
                screen.selection_key(&view, &key)
                    .or_else(|| screen.panel_key(&view, &key))
                    .unwrap_or_else(|| screen.key(key))
                }
            }
            _ => continue,
        };
        match out {
            KeyOut::Local => {},
            KeyOut::Quit => return Ok(()),
            KeyOut::Intent(mut intent) => {
                let draft=if let misa_proto::Intent::Prompt {text,attachments}=&mut intent {attachments.extend(pending.iter().cloned());Some(text.clone())}else{None};
                match session.send(intent).await {
                    Ok(())=>if draft.is_some() {pending.clear();screen.notice=None;},
                    Err(error)=>{if let Some(draft)=draft {screen.editor.set_text(&draft);}screen.notice=Some(error);}
                }
            },
            KeyOut::Complete { source, prefix } => match session.complete(&source, &prefix).await {
                Ok((items, truncated)) => screen.candidates(&source, items, truncated),
                Err(error) => screen.notice = Some(error),
            },
            KeyOut::Save(request) => screen.notice = Some(match crate::save::target(&view, &request) {
                Ok(node) => match session.save_attachment(node, &request.destination).await {
                    Ok(()) => format!("Saved {}", request.destination), Err(error) => error,
                }, Err(error) => error,
            }),
            KeyOut::Copy(text) => {
                use base64::Engine as _;
                let encoded = base64::engine::general_purpose::STANDARD.encode(text);
                write!(writer, "\x1b]52;c;{encoded}\x07").map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{Node, Intent, SessionInfo};
    use misa_proto::view::Choice;
    struct Idle { first: bool }
    #[async_trait::async_trait]
    impl Session for Idle {
        async fn next(&mut self) -> Result<Option<Node>, String> {
            if self.first { self.first = false; return Ok(Some(Node::section("session").id("session"))); }
            std::future::pending().await
        }
        async fn send(&mut self, _: Intent) -> Result<(), String> { Ok(()) }
        async fn complete(&mut self, _: &str, _: &str) -> Result<(Vec<Choice>, bool), String> { Ok((vec![], false)) }
        fn info(&self) -> Option<SessionInfo> { None }
    }
    #[tokio::test]
    async fn an_idle_session_still_accepts_paste_resize_and_quit() {
        let mut session = Idle { first: true };
        let mut screen = Screen::new(80, 24);
        let (sender, receiver) = mpsc::unbounded_channel();
        sender.send(Ok(Event::Paste("two\nlines".into()))).unwrap();
        sender.send(Ok(Event::Resize(100, 30))).unwrap();
        sender.send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Char('q'), event::KeyModifiers::CONTROL)))).unwrap();
        let mut output = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), drive(&mut session, &mut screen, receiver, &mut output, &mut crate::clipboard::Desktop))
            .await.expect("idle session blocked local input").unwrap();
        assert_eq!(screen.editor.text(), "two\nlines");
        assert_eq!((screen.width, screen.height), (100, 30));
    }
    struct ImageClipboard;
    impl crate::clipboard::Source for ImageClipboard {
        fn read(&mut self)->Result<crate::clipboard::Contents,String> {
            Ok(crate::clipboard::Contents::Image {width:1,height:1,rgba:vec![255,0,0,255]})
        }
    }
    struct UploadSession {first:bool,fail:bool,uploads:usize,sent:Vec<Intent>}
    #[async_trait::async_trait]
    impl Session for UploadSession {
        async fn next(&mut self)->Result<Option<Node>,String> {
            if self.first {self.first=false;Ok(Some(Node::section("session").id("session")))}else{std::future::pending().await}
        }
        async fn send(&mut self,intent:Intent)->Result<(),String> {
            self.sent.push(intent);
            if self.fail {self.fail=false;Err("retry send".into())}else{Ok(())}
        }
        async fn upload(&mut self,bytes:Vec<u8>,media:&str)->Result<misa_proto::view::BlobRef,String> {
            assert_eq!(media,"image/png");assert_eq!(image::load_from_memory(&bytes).unwrap().into_rgba8().into_raw(),vec![255,0,0,255]);
            self.uploads+=1;
            Ok(misa_proto::view::BlobRef {hash:format!("blob-{}",self.uploads),len:bytes.len() as u64,media:Some(media.into())})
        }
        async fn complete(&mut self,_:&str,_:&str)->Result<(Vec<Choice>,bool),String>{Ok((vec![],false))}
        fn info(&self)->Option<SessionInfo>{None}
    }
    fn key(code:event::KeyCode,modifiers:event::KeyModifiers)->Result<Event,String>{Ok(Event::Key(event::KeyEvent::new(code,modifiers)))}
    #[tokio::test]
    async fn pasted_image_is_staged_without_submitting() {
        let mut session=UploadSession {first:true,fail:false,uploads:0,sent:vec![]};
        let mut screen=Screen::new(80,24);
        let (sender,receiver)=mpsc::unbounded_channel();
        sender.send(key(event::KeyCode::Char('v'),event::KeyModifiers::CONTROL)).unwrap();
        sender.send(key(event::KeyCode::Char('q'),event::KeyModifiers::CONTROL)).unwrap();
        let mut output=vec![];
        drive(&mut session,&mut screen,receiver,&mut output,&mut ImageClipboard).await.unwrap();
        assert_eq!(session.uploads,1);assert!(session.sent.is_empty());
        assert!(String::from_utf8(output).unwrap().contains("1 clipboard attachments"));
    }
    #[tokio::test]
    async fn failed_submission_retains_attachments_and_success_consumes_them() {
        let mut session=UploadSession {first:true,fail:true,uploads:0,sent:vec![]};
        let mut screen=Screen::new(80,24);screen.editor.set_text("a picture");
        let (sender,receiver)=mpsc::unbounded_channel();
        for event in [key(event::KeyCode::Char('v'),event::KeyModifiers::CONTROL),key(event::KeyCode::Enter,event::KeyModifiers::NONE),key(event::KeyCode::Enter,event::KeyModifiers::NONE),key(event::KeyCode::Char('v'),event::KeyModifiers::CONTROL),key(event::KeyCode::Char('v'),event::KeyModifiers::CONTROL|event::KeyModifiers::ALT),key(event::KeyCode::Enter,event::KeyModifiers::NONE),key(event::KeyCode::Char('q'),event::KeyModifiers::CONTROL)] {sender.send(event).unwrap();}
        drive(&mut session,&mut screen,receiver,&mut Vec::new(),&mut ImageClipboard).await.unwrap();
        assert_eq!(session.sent.len(),2);assert_eq!(session.sent[0],session.sent[1]);
        assert!(matches!(&session.sent[1],Intent::Prompt {text,attachments} if text=="a picture" && attachments.len()==1 && attachments[0].hash=="blob-1"));
        assert_eq!(session.uploads,2);assert!(screen.editor.text().is_empty());
    }

}
