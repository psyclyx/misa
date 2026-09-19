use misa_proto::view::{Field, NodeId};
use misa_value::Value;

/// Local domain input adapter for owner handlers and fixtures.
/// Network callers use installed commands with trusted caller context.
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    /// Interrupt the current turn and submit this prompt before queued prompts.
    Interrupt {
        text: String,
        attachments: Vec<misa_proto::view::BlobRef>,
    },
    /// Submit a turn.
    Prompt {
        text: String,
        attachments: Vec<misa_proto::view::BlobRef>,
    },
    /// Resolve an action a view node offered.
    Action {
        node: NodeId,
        action: String,
        args: Value,
        /// Field values from the node, for an action with
        /// [`ActionOn::Submit`](misa_proto::view::ActionOn).
        fields: Vec<Field>,
    },
    /// Invoke a declared command by name.
    Command { name: String, args: Value },
}
