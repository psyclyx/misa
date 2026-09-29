//! Convert client-owned transactions into UI-owned incremental document changes.
use misa_tui_app::{Screen, retained::Retained};
pub fn observed(
    retained: &mut Retained,
    update: &misa_client::document::Update,
    screen: &Screen,
) -> Result<(), String> {
    use misa_client::document::Update;
    use misa_protocol::observation::{Applied, MemberChange};
    let kind = match update {
        Update::Reset(_) => "reset",
        Update::Changed { .. } => "changed",
        Update::Unavailable(_) => "unavailable",
        Update::Status(_) => "status",
    };
    misa_terminal_ui::trace::log(&format!("document update kind={kind}"));
    match update {
        Update::Reset(document) => {
            misa_terminal_ui::trace::log(&format!(
                "reset rows={} streams={}",
                document.tree.children.len(),
                document.streams.len()
            ));
            retained.reset(document.tree.clone(), &document.streams, screen)
        }
        Update::Changed { member, applied } => {
            let Applied::Changed(members) = applied.as_ref() else {
                return Err("Expected document transaction".into());
            };
            let Some(MemberChange::Document {
                tree,
                live,
                reset_live,
            }) = members.get(member)
            else {
                return Err("Expected document member changes".into());
            };
            retained.changed(tree, live, *reset_live, screen)?;
        }
        Update::Unavailable(_) | Update::Status(_) => {}
    }
    Ok(())
}
