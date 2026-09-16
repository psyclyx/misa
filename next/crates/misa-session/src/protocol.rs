//! Adapt this domain owner to coherent scoped reads and observations.
use crate::Runtime;
use misa_proto::Fault;

impl misa_protocol::owner::Owner for Runtime {
    fn read(&self, context: &misa_protocol::invocation::CallContext, selection: &misa_proto::observation::Selection) -> Result<misa_proto::observation::Snapshot, Fault> {
        self.read_selection_with_context(Some(context), selection)
    }
    fn observe(self: std::sync::Arc<Self>, context: misa_protocol::invocation::CallContext,
        handle: misa_proto::observation::Handle, selection: misa_proto::observation::Selection, resume: Option<misa_proto::observation::Resume>,
    ) -> Result<(Box<dyn misa_protocol::owner::Observation>, misa_proto::observation::Publication), Fault> {
        let (observation, publication) = Runtime::observe_with_context(&self, Some(context), handle, selection, resume)?;
        Ok((Box::new(observation), publication))
    }
}
