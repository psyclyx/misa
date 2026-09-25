//! Connected terminal host.
mod attachment_file;
mod dialogs;
mod document_adapter;
mod event_loop;
pub mod print;
pub mod save;
#[cfg(test)]
mod save_transport_tests;
pub mod scoped_remote;
pub mod storage;
pub mod workspace;
pub use misa_tui_ui::{
    Action, Catalog, Key, KeyOut, PanelInput, chrome, graphics, output, prefs, presentation,
    retained, terminal_loop,
};
pub mod clipboard;
use misa_kit::intent::Intent;
use misa_proto::{Node, view::Choice};
pub use misa_tui_ui::buttons;
pub use misa_tui_ui::{Accept, Picker, ed, panel_of, translate};
/// Connected screen state: independent rendering and host-owned request drafts.
pub struct Screen {
    pub ui: misa_tui_ui::Screen,
    pub dialogs: dialogs::Dialogs,
}
impl std::ops::Deref for Screen {
    type Target = misa_tui_ui::Screen;
    fn deref(&self) -> &Self::Target {
        &self.ui
    }
}
impl std::ops::DerefMut for Screen {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ui
    }
}
impl Screen {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            ui: misa_tui_ui::Screen::new(width, height),
            dialogs: Default::default(),
        }
    }
    pub fn durable() -> Self {
        Self {
            ui: {
                let path = storage::File::default_path();
                let file = storage::File::at(path);
                misa_tui_ui::Screen::remembering(prefs::Prefs::load(&file), Box::new(file))
            },
            dialogs: Default::default(),
        }
    }
    pub fn remembering(prefs: prefs::Prefs, path: std::path::PathBuf) -> Self {
        Self {
            ui: misa_tui_ui::Screen::remembering(prefs, Box::new(storage::File::at(path))),
            dialogs: Default::default(),
        }
    }
    /// Install the session catalog and the connected host's local file command.
    /// A session's declaration with the same id always takes precedence.
    pub fn declare(&mut self, catalog: &Catalog) {
        self.ui.declare_with_raw(
            catalog,
            &[misa_kit::intent::Command::new(
                "save",
                "Save attachment",
                "/save [number] <local path>",
            )],
        );
    }
    pub fn refresh_dialog_surface(&mut self) {
        self.ui.dialogs = misa_tui_ui::DialogSurface {
            modal: self.dialogs.modal(),
            content: self.dialogs.lines(
                &self.ui.theme,
                self.ui.width as usize,
                &self.ui.prefs.dialogs,
            ),
        };
    }
}

/// What a frontend needs from a transport, so this binary can be tested and the
/// transport can be swapped.
#[async_trait::async_trait]
pub trait Session: Send {
    /// Scoped clients correlate completion to the accepted operation.
    fn turn_settled(&self) -> Option<bool> {
        None
    }
    /// The next view, if one changed.
    async fn next(&mut self) -> Result<Option<Node>, String>;
    /// Incremental consumers retain presentation owners between these updates.
    async fn next_presentation(&mut self) -> Result<Option<Presentation>, String> {
        Ok(self.next().await?.map(Presentation::Snapshot))
    }
    async fn request(&mut self, request: SessionRequest) -> Option<SessionReply> {
        Some(match request {
            SessionRequest::RefreshRequests => {
                SessionReply::Notice("This session has no input request catalog".into())
            }
            SessionRequest::DaemonInvoke { .. } | SessionRequest::Invoke { .. } => {
                SessionReply::Notice("This session does not support installed invocations".into())
            }
            SessionRequest::Intent(intent) => {
                let draft = match &intent {
                    Intent::Prompt { text, attachments }
                    | Intent::Interrupt { text, attachments } => {
                        Some((text.clone(), attachments.clone()))
                    }
                    _ => None,
                };
                SessionReply::Sent {
                    draft,
                    result: self.send(intent).await,
                }
            }
            SessionRequest::Upload {
                generation,
                bytes,
                media,
            } => SessionReply::Uploaded {
                generation,
                result: self.upload(bytes, &media).await,
            },
            SessionRequest::Download { reference } => SessionReply::Downloaded {
                reference: reference.clone(),
                result: self.download(&reference).await,
            },
            SessionRequest::Complete { source, prefix } => {
                let result = self.complete(&source, &prefix).await;
                SessionReply::Complete {
                    source,
                    prefix,
                    result,
                }
            }
            SessionRequest::Save { node, destination } => {
                SessionReply::Notice(match self.save_attachment(&node, &destination).await {
                    Ok(()) => format!("Saved {destination}"),
                    Err(error) => error,
                })
            }
        })
    }
    async fn send(&mut self, intent: Intent) -> Result<(), String>;
    async fn upload(
        &mut self,
        _bytes: Vec<u8>,
        _media: &str,
    ) -> Result<misa_proto::view::BlobRef, String> {
        Err("This client has no blob connection".into())
    }
    async fn save_attachment(&mut self, _node: &str, _destination: &str) -> Result<(), String> {
        Err("This client has no blob connection".into())
    }
    /// Fetch the bytes a [`misa_proto::view::BlobRef`] names. The generic client
    /// renders a placeholder until it returns, so an implementation is free to be
    /// slow — but it must not be called on the render path.
    async fn download(
        &mut self,
        _reference: &misa_proto::view::BlobRef,
    ) -> Result<Vec<u8>, String> {
        Err("This client has no blob connection".into())
    }
    /// Candidates a session holds, for a source this client asked about.
    async fn complete(&mut self, source: &str, prefix: &str)
    -> Result<(Vec<Choice>, bool), String>;
    fn catalog(&self) -> Catalog {
        Catalog::default()
    }
    fn location(&self) -> String {
        "Session".into()
    }
    fn selected(&self) -> Option<misa_proto::directory::Entry> {
        None
    }
}

pub enum Presentation {
    Attention {
        id: String,
        generation: i64,
    },
    Documents(Vec<(String, misa_client::document::Update)>),
    Activate(String),
    Forget(String),
    Contribution {
        id: String,
        update: misa_client::document::Update,
    },
    TurnOutput(Node),
    Document(misa_client::document::Update),
    Declaration {
        catalog: Catalog,
        location: String,
    },
    Candidates {
        source: String,
        items: Vec<Choice>,
        truncated: bool,
    },
    Snapshot(Node),
    Reply(SessionReply),
}
pub enum SessionRequest {
    DaemonInvoke {
        daemon: String,
        scope: misa_proto::observation::Scope,
        command: String,
        input: misa_value::Value,
    },
    RefreshRequests,
    Invoke {
        command: String,
        input: misa_value::Value,
    },
    Intent(Intent),
    Upload {
        generation: u64,
        bytes: Vec<u8>,
        media: String,
    },
    Download {
        reference: misa_proto::view::BlobRef,
    },
    Complete {
        source: String,
        prefix: String,
    },
    Save {
        node: String,
        destination: String,
    },
}
pub enum SessionReply {
    DaemonForm {
        daemon: String,
        scope: misa_proto::observation::Scope,
        form: misa_client::form::Form,
        drafts: std::collections::BTreeMap<String, String>,
    },
    Form(misa_client::form::Form),
    Request {
        id: String,
        generation: i64,
        model: Option<misa_client::request::Model>,
    },
    Report(Node),
    Uploaded {
        generation: u64,
        result: Result<misa_proto::view::BlobRef, String>,
    },
    Downloaded {
        reference: misa_proto::view::BlobRef,
        result: Result<Vec<u8>, String>,
    },
    Sent {
        draft: Option<(String, Vec<misa_proto::view::BlobRef>)>,
        result: Result<(), String>,
    },
    Complete {
        source: String,
        prefix: String,
        result: Result<(Vec<Choice>, bool), String>,
    },
    Notice(String),
}

/// The interactive loop.
pub async fn run(session: &mut dyn Session) -> Result<(), String> {
    event_loop::run(session).await
}

#[cfg(test)]
fn test_unique_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed) + std::process::id() as u64 * 100_000
}
