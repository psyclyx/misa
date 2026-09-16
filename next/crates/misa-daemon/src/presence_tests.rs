//! Presence belongs to authenticated daemon sockets, without selecting a session.
use crate::directory::{Directory, Routes};
use misa_proto::{ClientInfo, Query, observation::*, scoped::*};
use misa_protocol::{
    invocation::{CallContext, CommandOwner},
    owner::Owner,
};
use misa_transport::{
    admission::{Admission, Paired},
    scoped_client::{Client, Event},
};
use std::{sync::Arc, time::Duration};
struct EmptyBlobs;
impl misa_transport::blob::BlobStore for EmptyBlobs {
    fn get(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
    fn media(&self, _: &str) -> Option<String> {
        None
    }
    fn has(&self, _: &str) -> bool {
        false
    }
    fn store(&self, _: Vec<u8>, _: Option<&str>) -> Result<misa_proto::view::BlobRef, String> {
        Err("read only".into())
    }
}
#[tokio::test]
async fn two_real_connections_are_independent_of_sessions_and_retire_on_disconnect_or_revocation() {
    let directory = Directory::new("presence").unwrap();
    let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
    let first_endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
    let second_endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
    let paired = Paired::in_memory();
    paired
        .add(&first_endpoint.id().to_string(), "one", 0)
        .unwrap();
    paired
        .add(&second_endpoint.id().to_string(), "two", 0)
        .unwrap();
    let admission = Arc::new(Admission::paired(paired));
    let address =
        misa_transport::iroh::address_of(&misa_transport::iroh::node_of(&endpoint)).unwrap();
    let router = misa_transport::server::serve(
        endpoint.clone(),
        Arc::new(EmptyBlobs),
        admission.clone(),
        misa_transport::scoped_server::Handler {
            daemon: endpoint.id().to_string(),
            scope: directory.scope(),
            resolver: Arc::new(Routes(directory.clone())),
            admission: admission.clone(),
        },
    );
    let mut first = Client::connect(
        &first_endpoint,
        address.clone(),
        ClientInfo::new("terminal", "1"),
    )
    .await
    .unwrap();
    let second = Client::connect(&second_endpoint, address, ClientInfo::new("phone", "2"))
        .await
        .unwrap();
    let selection = Selection {
        scope: directory.scope(),
        members: std::collections::BTreeMap::from([(
            "connections".into(),
            Member {
                query: Query::new("daemon.connections"),
                contract: "daemon.connections@1".into(),
                encoding: Encoding::Value,
                optional: false,
            },
        )]),
    };
    first
        .try_send(ClientMessage::Read {
            read: Read {
                id: 1,
                selection: selection.clone(),
            },
        })
        .unwrap();
    let Event::Message(frame) = tokio::time::timeout(Duration::from_secs(5), first.next())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!()
    };
    let ServerMessage::ReadReply {
        reply:
            ReadReply {
                outcome: ReadOutcome::Snapshot { snapshot },
                ..
            },
    } = frame.message
    else {
        panic!()
    };
    let Content::Value(rows) = &snapshot.members["connections"] else {
        panic!()
    };
    assert_eq!(rows.as_list().unwrap().len(), 2);
    assert!(
        directory.sessions().is_empty(),
        "connections never attach or create sessions"
    );
    let context = CallContext {
        principal: "test".into(),
        connection: 0,
    };
    let mut changes = directory.watch_work();
    drop(second);
    for expected in [1, 0] {
        if expected == 0 {
            admission.revoke(&first_endpoint.id().to_string()).unwrap();
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let snapshot = directory.read(&context, &selection).unwrap();
                let Content::Value(rows) = &snapshot.members["connections"] else {
                    panic!()
                };
                if rows.as_list().unwrap().len() == expected {
                    break;
                }
                changes.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }
    drop(first);
    directory.shutdown_complete().await;
    router.shutdown().await.unwrap();
    first_endpoint.close().await;
    second_endpoint.close().await;
}
