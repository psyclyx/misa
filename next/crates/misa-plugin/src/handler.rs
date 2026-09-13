//! A plugin, as a handler the loop runs.

use std::sync::Arc;

use misa_reframe::{Event, Fault, Handler, Tx};

use crate::host::Plugin;

/// Where a plugin's handlers sit in the order for one event kind.
///
/// After the session's own, which are registered at zero: a plugin's patches land on top of
/// what the loop decided rather than underneath it, and a plugin cannot shadow a built-in
/// handler's decision. Ordering is a composition's business, so it is one number in one place.
pub const PLUGIN_PRIORITY: i32 = 10;

/// One event kind of one plugin.
///
/// The handler holds the *plugin* rather than an instance of the bindings, so every handler a
/// plugin contributes shares its store: what a plugin remembers between events is its own
/// business, and one call at a time is the rule that makes it safe.
pub struct PluginHandler {
    plugin: Arc<Plugin>,
    /// The event kind this handler was registered for.
    kind: String,
    id: String,
}

impl PluginHandler {
    pub fn new(plugin: Arc<Plugin>, kind: String) -> PluginHandler {
        let id = format!("plugin.{}.{}", plugin.descriptor().id, kind);
        PluginHandler { plugin, kind, id }
    }

    pub fn plugin(&self) -> &Arc<Plugin> {
        &self.plugin
    }
}

impl Handler for PluginHandler {
    fn id(&self) -> &str {
        &self.id
    }

    /// One event: the plugin is asked, and what it answers is applied.
    ///
    /// The database it is handed is the same one this handler's own `tx` reads, which is the
    /// loop's rule rather than an exception to it — `tx.db()` is the committed state, not the
    /// patches other handlers have queued in this transaction. A plugin sees the world as it
    /// was when the event arrived, exactly like every other handler.
    ///
    /// A fault from the plugin — its own, or the host's refusal of something it sent — is the
    /// handler's fault, so the transaction rolls back and the reason is reported. Nothing a
    /// plugin does can leave half a patch behind.
    fn handle(&self, tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
        let (patches, effects) = self.plugin.handle(event, tx.db()).map_err(|fault| {
            Fault::handler(format!(
                "`{}` while handling `{}`: {}",
                self.plugin.descriptor().id,
                self.kind,
                fault.message
            ))
        })?;
        for patch in patches {
            tx.patch(&patch.path, patch.op)?;
        }
        for effect in effects {
            tx.fx(effect);
        }
        Ok(())
    }
}
