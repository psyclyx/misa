use crate::{
    admission::Admission,
    scoped_client::{Client, Event},
};
use misa_proto::{
    ClientInfo, Fault, Query,
    invocation::{Invocation, Outcome},
    observation::{Content, Encoding, Handle, Member, Scope, ScopeId, Selection},
    scoped::{ClientMessage, ServerMessage},
};
use misa_protocol::{
    invocation::CallContext,
    observation::{MemberState, Replica},
    owner::{Owner, Resolver},
};
use misa_value::Value;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

struct Registry(Arc<misa_session::Runtime>);
impl Resolver for Registry {
    fn resolve(&self, _: &CallContext, scope: &Scope) -> Result<Arc<dyn Owner>, Fault> {
        if *scope == self.0.scope() {
            Ok(self.0.clone())
        } else {
            Err(Fault::new("scope_unavailable", "Scope unavailable"))
        }
    }
}
fn selection(runtime: &misa_session::Runtime, query: &str) -> Selection {
    Selection {
        scope: runtime.scope(),
        members: BTreeMap::from([(
            "document".into(),
            Member {
                query: Query::new(query),
                contract: format!("{query}@1"),
                encoding: Encoding::Document,
                optional: false,
            },
        )]),
    }
}
async fn next(client: &mut Client) -> ServerMessage {
    match tokio::time::timeout(Duration::from_secs(10), client.next())
        .await
        .unwrap()
        .unwrap()
    {
        Event::Message(frame) => frame.message,
        Event::LaneClosed { reason, .. } => panic!("lane closed: {reason}"),
    }
}
async fn fixture(
    model: &str,
) -> (
    iroh::protocol::Router,
    iroh::Endpoint,
    iroh::EndpointAddr,
    Arc<misa_session::Runtime>,
) {
    let server = crate::iroh::bind(None, false).await.unwrap();
    let endpoint = crate::iroh::bind(None, false).await.unwrap();
    let runtime = misa_session::Runtime::start(
        "demo",
        "Demo",
        None,
        Arc::new(misa_kernel::LocalKernel::new(
            misa_kernel::ScriptedProvider::always("network result"),
        )),
        "scripted",
        model,
        Value::Null,
    );
    let address = crate::iroh::address_of(&crate::iroh::node_of(&server)).unwrap();
    let router = iroh::protocol::Router::builder(server.clone())
        .accept(
            misa_proto::scoped::ALPN,
            crate::scoped_server::Handler {
                daemon: server.id().to_string(),
                scope: Scope {
                    id: ScopeId::Daemon,
                    incarnation: "test".into(),
                },
                resolver: Arc::new(Registry(runtime.clone())),
                admission: Arc::new(Admission::open()),
            },
        )
        .spawn();
    (router, endpoint, address, runtime)
}
async fn connect(endpoint: &iroh::Endpoint, address: iroh::EndpointAddr) -> Client {
    Client::connect(endpoint, address, ClientInfo::new("test", "1"))
        .await
        .unwrap()
}
fn document(replica: &Replica) -> misa_proto::observation::Document {
    match &replica.current().unwrap()["document"] {
        MemberState::Document(v) => v.snapshot(),
        _ => panic!("document required"),
    }
}
#[tokio::test]
async fn large_canonical_document_crosses_real_endpoint_without_truncation() {
    let (router, endpoint, address, runtime) =
        fixture(&"x".repeat(misa_proto::MAX_CONTROL_FRAME + 1024)).await;
    let selection = selection(&runtime, "status.presentation");
    let expected_snapshot = runtime.read_selection(&selection).unwrap();
    let Content::Document(expected) = &expected_snapshot.members["document"] else {
        panic!()
    };
    assert!(
        misa_proto::frame::encode_within(&expected.tree, usize::MAX)
            .unwrap()
            .len()
            > misa_proto::MAX_CONTROL_FRAME
    );
    let mut client = connect(&endpoint, address).await;
    let handle = Handle {
        id: 1,
        generation: 1,
    };
    client
        .try_send(ClientMessage::Observe {
            handle,
            selection: selection.clone(),
            resume: None,
        })
        .unwrap();
    let ServerMessage::Publication { publication } = next(&mut client).await else {
        panic!()
    };
    let mut expected_replica = Replica::new(handle, selection.clone()).unwrap();
    expected_replica
        .apply(misa_proto::observation::Publication::Snapshot {
            handle,
            snapshot: expected_snapshot,
        })
        .unwrap();
    let mut replica = Replica::new(handle, selection).unwrap();
    replica.apply(publication).unwrap();
    assert!(
        document(&replica).tree == document(&expected_replica).tree,
        "complete normalized canonical document differs"
    );
    drop(client);
    runtime.shutdown_complete().await;
    router.shutdown().await.unwrap();
    endpoint.close().await;
}
#[tokio::test]
async fn two_clients_converge_and_reconnect_resumes_the_same_owned_document() {
    let (router, endpoint, address, runtime) = fixture("scripted-1").await;
    let selection = selection(&runtime, "conversation.presentation");
    let handle = Handle {
        id: 1,
        generation: 1,
    };
    let mut a = connect(&endpoint, address.clone()).await;
    let endpoint_b = crate::iroh::bind(None, false).await.unwrap();
    let mut b = connect(&endpoint_b, address.clone()).await;
    let mut replicas = [
        Replica::new(handle, selection.clone()).unwrap(),
        Replica::new(handle, selection.clone()).unwrap(),
    ];
    for (client, replica) in [(&mut a, &mut replicas[0])].into_iter() {
        client
            .try_send(ClientMessage::Observe {
                handle,
                selection: selection.clone(),
                resume: None,
            })
            .unwrap();
        let ServerMessage::Publication { publication } = next(client).await else {
            panic!()
        };
        replica.apply(publication).unwrap();
    }
    b.try_send(ClientMessage::Observe {
        handle,
        selection: selection.clone(),
        resume: None,
    })
    .unwrap();
    let ServerMessage::Publication { publication } = next(&mut b).await else {
        panic!()
    };
    replicas[1].apply(publication).unwrap();
    assert_eq!(document(&replicas[0]).tree, document(&replicas[1]).tree);
    a.try_send(ClientMessage::Invoke {
        invocation: Invocation {
            id: 2,
            scope: runtime.scope(),
            command: "session.prompt".into(),
            input: Value::map([("text", Value::str("hello"))]),
        },
    })
    .unwrap();
    for (client, replica) in [&mut a, &mut b].into_iter().zip(replicas.iter_mut()) {
        loop {
            match next(client).await {
                ServerMessage::Publication { publication } => {
                    replica.apply(publication).unwrap();
                }
                ServerMessage::Reply { reply } => {
                    assert!(matches!(reply.outcome, Outcome::Accepted { .. }))
                }
                other => panic!("unexpected {other:?}"),
            }
            let doc = document(replica);
            if doc.streams.is_empty() && format!("{:?}", doc.tree).contains("network result") {
                break;
            }
        }
    }
    assert_eq!(document(&replicas[0]).tree, document(&replicas[1]).tree);
    drop(a);
    let resume = replicas[0].resume();
    let new_handle = Handle {
        id: 1,
        generation: 2,
    };
    replicas[0].mark_disconnected();
    replicas[0].rebind(new_handle).unwrap();
    let mut a = connect(&endpoint, address).await;
    a.try_send(ClientMessage::Observe {
        handle: new_handle,
        selection,
        resume: Some(resume),
    })
    .unwrap();
    let ServerMessage::Publication { publication } = next(&mut a).await else {
        panic!()
    };
    assert_eq!(publication.handle(), new_handle);
    replicas[0].apply(publication).unwrap();
    assert_eq!(document(&replicas[0]).tree, document(&replicas[1]).tree);
    drop(a);
    drop(b);
    runtime.shutdown_complete().await;
    router.shutdown().await.unwrap();
    endpoint.close().await;
    endpoint_b.close().await;
}
