//! Owner admission precedes publication. Reservations follow deferred effects
//! and running requests, so slow capabilities cannot create an unbounded backlog.
use misa_kernel::{Kernel, KernelEvent, Request};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, watch};

pub(crate) const EXTERNAL: usize = 256;
pub(crate) const CONTROL: usize = 256;
const MAX_EFFECTS: usize = 128;
#[derive(Clone, Copy)]
pub(crate) enum Class {
    External,
    Control,
}
#[derive(Clone)]
pub(crate) struct Budget {
    external: Arc<Semaphore>,
    control: Arc<Semaphore>,
    pub wake: Arc<Notify>,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            external: Arc::new(Semaphore::new(EXTERNAL)),
            control: Arc::new(Semaphore::new(CONTROL)),
            wake: Arc::new(Notify::new()),
        }
    }
}
impl Budget {
    pub fn reserve(&self, class: Class, effects: usize) -> Result<Admission, misa_reframe::Fault> {
        if effects > MAX_EFFECTS {
            return Err(misa_reframe::Fault::new(
                "admission.limit",
                "Transaction asks for too many effects",
            ));
        }
        // One checkpoint and one private operation request can follow finalization.
        let slots = effects + 2;
        let pool = match class {
            Class::External => &self.external,
            Class::Control => &self.control,
        };
        let permit = pool
            .clone()
            .try_acquire_many_owned(slots as u32)
            .map_err(|_| {
                misa_reframe::Fault::new("admission.busy", "Owner capability work capacity is full")
            })?;
        Ok(Admission(vec![Arc::new(Lease {
            permit: Some(permit),
            free: AtomicUsize::new(slots),
            wake: self.wake.clone(),
        })]))
    }
    #[cfg(test)]
    pub fn available(&self) -> (usize, usize) {
        (
            self.external.available_permits(),
            self.control.available_permits(),
        )
    }
}
struct Lease {
    permit: Option<OwnedSemaphorePermit>,
    free: AtomicUsize,
    wake: Arc<Notify>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        drop(self.permit.take());
        self.wake.notify_one();
    }
}
#[derive(Clone, Default)]
pub(crate) struct Admission(Vec<Arc<Lease>>);
impl Admission {
    pub fn extend(&mut self, other: Self) {
        self.0.extend(other.0);
    }
    fn take(&self) -> Result<Ticket, String> {
        for lease in &self.0 {
            if lease
                .free
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |free| {
                    free.checked_sub(1)
                })
                .is_ok()
            {
                return Ok(Ticket(lease.clone()));
            }
        }
        Err("Committed effect exceeded its reserved capability budget".into())
    }
}
struct Ticket(Arc<Lease>);
impl Drop for Ticket {
    fn drop(&mut self) {
        self.0.free.fetch_add(1, Ordering::Release);
    }
}
pub(crate) struct Outcome {
    pub inner: misa_reframe::Outcome,
    pub admission: Admission,
}
impl std::ops::Deref for Outcome {
    type Target = misa_reframe::Outcome;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl std::ops::DerefMut for Outcome {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
struct Envelope {
    request: Request,
    _ticket: Ticket,
}
#[derive(Clone)]
pub(crate) struct Queue {
    work: mpsc::UnboundedSender<Envelope>,
    log: mpsc::UnboundedSender<Envelope>,
}
pub(crate) struct Receiver {
    work: mpsc::UnboundedReceiver<Envelope>,
    log: mpsc::UnboundedReceiver<Envelope>,
}
impl Queue {
    pub fn new() -> (Self, Receiver) {
        let (work, work_rx) = mpsc::unbounded_channel();
        let (log, log_rx) = mpsc::unbounded_channel();
        (
            Self { work, log },
            Receiver {
                work: work_rx,
                log: log_rx,
            },
        )
    }
    pub fn send(&self, request: Request, admission: &Admission) -> Result<(), String> {
        let ticket = admission.take()?;
        let channel = if matches!(&request, Request::Append { .. } | Request::Load { .. }) {
            &self.log
        } else {
            &self.work
        };
        channel
            .send(Envelope {
                request,
                _ticket: ticket,
            })
            .map_err(|_| "Kernel dispatcher closed".into())
    }
}
impl Receiver {
    pub async fn run(
        mut self,
        kernel: Arc<dyn Kernel>,
        events: mpsc::UnboundedSender<KernelEvent>,
        mut closing: watch::Receiver<bool>,
        stopped: watch::Sender<bool>,
    ) {
        let mut tasks = tokio::task::JoinSet::new();
        let log_kernel = kernel.clone();
        let log_events = events.clone();
        // Journal ordering has its own serial lane; blocked file/provider jobs
        // cannot prevent a durable acknowledgment or cancellation from running.
        tasks.spawn(async move {
            while let Some(envelope) = self.log.recv().await {
                log_kernel.execute(envelope.request, &log_events).await;
                drop(envelope._ticket);
            }
        });
        let workers = Arc::new(Semaphore::new(64));
        let controls = Arc::new(Semaphore::new(16));
        loop {
            tokio::select! {
                biased;
                _=closing.changed()=>break,
                _=tasks.join_next(),if !tasks.is_empty()=>{},
                request=self.work.recv()=>{
                    let Some(envelope)=request else{break;};
                    let pool=if matches!(&envelope.request,Request::AttemptStarted{..}|Request::AttemptSettled{..}|Request::Credential{..}){controls.clone()}else{workers.clone()};
                    let kernel=kernel.clone();let out=events.clone();
                    // Waiting tasks are bounded by the same reserved tickets; the
                    // semaphore only limits executing capability calls, not admission.
                    tasks.spawn(async move{let _worker=pool.acquire_owned().await.expect("worker pool lives with dispatcher");kernel.execute(envelope.request,&out).await;drop(envelope._ticket);});
                }
            }
        }
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        // Dropping receivers releases every queued request's reservation.
        drop(self.work);
        kernel.close(&events).await;
        stopped.send_replace(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Contribution, Runtime};
    use misa_kernel::CredentialAction;
    use misa_reframe::Event;
    use misa_value::Value;
    use std::{future::Future, pin::Pin, sync::atomic::AtomicUsize};
    #[derive(Default)]
    struct Blocked {
        files: AtomicUsize,
        logs: AtomicUsize,
        controls: AtomicUsize,
    }
    impl Kernel for Blocked {
        fn execute<'a, 'b, 'f>(
            &'a self,
            request: Request,
            out: &'b mpsc::UnboundedSender<KernelEvent>,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'f>>
        where
            'a: 'f,
            'b: 'f,
            Self: 'f,
        {
            Box::pin(async move {
                match request {
                    Request::BlobFile { .. } => {
                        self.files.fetch_add(1, Ordering::SeqCst);
                        std::future::pending::<()>().await;
                    }
                    Request::Append {
                        conversation,
                        kind,
                        data,
                    } => {
                        let seq = self.logs.fetch_add(1, Ordering::SeqCst) as i64 + 1;
                        let _ = out.send(KernelEvent::Appended {
                            conversation,
                            kind,
                            data,
                            seq,
                        });
                    }
                    Request::Credential {
                        action: CredentialAction::CancelOAuth { .. },
                        ..
                    } => {
                        self.controls.fetch_add(1, Ordering::SeqCst);
                    }
                    Request::Credential {
                        id,
                        action: CredentialAction::Set { .. },
                    } => {
                        let _ = out.send(KernelEvent::Credential {
                            id,
                            ok: true,
                            message: String::new(),
                            slots: Value::list([]),
                        });
                    }
                    _ => {}
                }
            })
        }
    }
    async fn until(mut predicate: impl FnMut() -> bool) {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !predicate() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn saturated_external_transactions_rollback_and_control_and_journal_progress() {
        let kernel = Arc::new(Blocked::default());
        let runtime = Runtime::start_with(
            "bounded",
            "Bounded",
            None,
            kernel.clone(),
            "scripted",
            "test",
            Value::Null,
            Contribution::default(),
        );
        until(|| runtime.kernel_budget.available() == (EXTERNAL, CONTROL)).await;
        let context = misa_protocol::invocation::CallContext {
            principal: "test".into(),
            connection: 1,
        };
        let invoke = |id, command: &str, input| {
            runtime.execute_command(
                &context,
                misa_proto::invocation::Invocation {
                    id,
                    scope: runtime.scope(),
                    command: command.into(),
                    input,
                },
            )
        };
        let misa_proto::invocation::Outcome::Accepted { operation } = invoke(
            1000,
            "credentials.authorize",
            Value::map([("provider", Value::str("openai"))]),
        ) else {
            panic!("credential operation refused")
        };
        until(|| runtime.kernel_budget.available() == (EXTERNAL, CONTROL)).await;
        let mut accepted = 0;
        loop {
            let before = {
                let state = runtime.state.lock().unwrap();
                (state.state.db().clone(), state.state.rev())
            };
            let result = runtime.execute_command(
                &misa_protocol::invocation::CallContext {
                    principal: "test".into(),
                    connection: 1,
                },
                misa_proto::invocation::Invocation {
                    id: accepted + 1,
                    scope: runtime.scope(),
                    command: "session.attachment.add".into(),
                    input: Value::map([("path", Value::str("blocked"))]),
                },
            );
            let faults = match result {
                misa_proto::invocation::Outcome::Rejected { fault } => vec![fault],
                misa_proto::invocation::Outcome::Completed { .. } => vec![],
                other => panic!("{other:?}"),
            };
            if !faults.is_empty() {
                assert_eq!(faults[0].code, "admission.busy");
                let state = runtime.state.lock().unwrap();
                assert_eq!(state.state.db(), &before.0);
                assert_eq!(state.state.rev(), before.1);
                break;
            }
            accepted += 1;
            assert!(accepted <= EXTERNAL as u64);
        }
        assert_eq!(accepted, EXTERNAL as u64 / 3);
        until(|| kernel.files.load(Ordering::SeqCst) == 64).await;
        let result = invoke(
            1001,
            "credentials.resolve",
            Value::map([
                ("request", Value::str(&operation.id)),
                ("generation", Value::Int(1)),
                ("value", Value::str("private-token")),
            ]),
        );
        assert!(matches!(
            result,
            misa_proto::invocation::Outcome::Accepted { .. }
        ));
        until(|| {
            let mut state = runtime.state.lock().unwrap();
            let result = state
                .state
                .query(&misa_proto::Query::new("operation.result").arg(Value::str(&operation.id)))
                .unwrap();
            result.get("state").and_then(Value::as_str) == Some("succeeded")
        })
        .await;
        assert_eq!(
            kernel.files.load(Ordering::SeqCst),
            64,
            "blocked files stayed blocked throughout checkpointed completion"
        );
        let logs_before = kernel.logs.load(Ordering::SeqCst);
        let guard = runtime.kernel_budget.reserve(Class::Control, 0).unwrap();
        runtime
            .to_kernel
            .send(
                Request::Append {
                    conversation: "bounded".into(),
                    kind: "test".into(),
                    data: Value::Null,
                },
                &guard,
            )
            .unwrap();
        runtime
            .to_kernel
            .send(
                Request::Credential {
                    id: "cancel".into(),
                    action: CredentialAction::CancelOAuth {
                        request: "existing".into(),
                    },
                },
                &guard,
            )
            .unwrap();
        until(|| {
            kernel.logs.load(Ordering::SeqCst) > logs_before
                && kernel.controls.load(Ordering::SeqCst) == 1
        })
        .await;
        drop(guard);
        runtime.shutdown();
        until(|| runtime.kernel_budget.available() == (EXTERNAL, CONTROL)).await;
        assert!(
            runtime
                .dispatch(Event::new("admission/test"))
                .iter()
                .any(|f| f.code == "closed_scope")
        );
    }
    #[test]
    fn deferred_reservations_live_until_final_guard_and_ticket_drop() {
        let budget = Budget::default();
        let guard = budget.reserve(Class::External, 2).unwrap();
        let deferred = guard.clone();
        let ticket = guard.take().unwrap();
        drop(guard);
        assert_eq!(budget.available().0, EXTERNAL - 4);
        drop(deferred);
        assert_eq!(budget.available().0, EXTERNAL - 4);
        drop(ticket);
        assert_eq!(budget.available().0, EXTERNAL);
        assert!(budget.reserve(Class::External, MAX_EFFECTS + 1).is_err());
    }
}
