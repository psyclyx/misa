//! Admitted owner execution outlives a waiting connection. Shutdown drains it.
use misa_proto::{Fault, invocation::Reply};
use std::{future::Future, sync::Mutex};
use tokio::{sync::oneshot, task::JoinSet};

const CAPACITY: usize = 128;

#[derive(Default)]
pub(crate) struct Invocations(Mutex<State>);
#[derive(Default)]
struct State {
    closed: bool,
    tasks: JoinSet<()>,
}
impl Invocations {
    pub(crate) fn spawn(
        &self,
        execution: impl Future<Output = Reply> + Send + 'static,
    ) -> Result<oneshot::Receiver<Reply>, Fault> {
        let mut state = self.0.lock().expect("invocation tasks");
        while state.tasks.try_join_next().is_some() {}
        if state.closed {
            return Err(Fault::new("closing", "Daemon command admission is closing"));
        }
        if state.tasks.len() >= CAPACITY {
            return Err(Fault::new("busy", "Daemon invocation capacity reached"));
        }
        let (send, receive) = oneshot::channel();
        state.tasks.spawn(async move {
            let reply = execution.await;
            // The caller may have disconnected after the owner began its write.
            // Losing the reply never cancels that write or triggers a replay.
            let _ = send.send(reply);
        });
        Ok(receive)
    }

    pub(crate) async fn shutdown(&self) {
        let mut tasks = {
            let mut state = self.0.lock().expect("invocation tasks");
            state.closed = true;
            std::mem::take(&mut state.tasks)
        };
        while tasks.join_next().await.is_some() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::invocation::Outcome;
    use misa_value::Value;

    #[tokio::test]
    async fn abandoned_callers_keep_admitted_work_bounded_until_completion() {
        let invocations = Invocations::default();
        let mut release = Vec::new();
        for id in 0..CAPACITY {
            let (send, receive) = oneshot::channel();
            release.push(send);
            drop(
                invocations
                    .spawn(async move {
                        receive.await.unwrap();
                        Reply {
                            id: id as u64,
                            outcome: Outcome::Completed { value: Value::Null },
                        }
                    })
                    .unwrap(),
            );
        }
        assert!(
            invocations
                .spawn(async { unreachable!("capacity refusal cannot execute") })
                .is_err()
        );
        for send in release {
            send.send(()).unwrap();
        }
        invocations.shutdown().await;
        assert!(
            invocations
                .spawn(async { unreachable!("shutdown refuses new execution") })
                .is_err()
        );
    }
}
