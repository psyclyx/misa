//! Convert client-owned transactions into UI-owned incremental document changes.
use misa_tui_ui::{Screen, retained::Retained};
pub fn observed(
    retained: &mut Retained,
    update: &misa_client::document::Update,
    screen: &Screen,
) -> Result<(), String> {
    use misa_client::document::Update;
    use misa_protocol::observation::{Applied, MemberChange};
    match update {
        Update::Reset(document) => retained.reset(document.tree.clone(), &document.streams, screen),
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
