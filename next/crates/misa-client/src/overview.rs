//! Cheap daemon summaries and work relationships, captured at one publication.
//! The owner computes rollups. Clients never traverse transcripts to recreate them.
use crate::{
    driver::{Client, Observation},
    interface::{self, Interface},
};
use misa_proto::{
    Fault,
    directory::{Availability, Entry},
    observation::{Scope, Selection},
};
use misa_protocol::observation::{MemberState, Status};
use misa_value::Value;
use serde::Deserialize;
use std::collections::BTreeMap;
#[derive(Clone, Debug, Deserialize)]
pub struct Usage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_micros: i64,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Attention {
    pub scope: Scope,
    pub source_position: u64,
    pub availability: Availability,
    pub request: Value,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Row {
    pub scope: Scope,
    pub source_position: u64,
    pub availability: Availability,
    pub direct_working: bool,
    pub working: bool,
    pub direct_attention: u64,
    pub attention: u64,
    pub requests: Vec<Attention>,
    pub blocking: Vec<String>,
    pub unavailable: Vec<Scope>,
    pub cyclic: bool,
    pub direct_usage: Usage,
    pub inclusive_usage: Usage,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Work {
    pub id: String,
    pub parent: Scope,
    pub child: Option<Scope>,
    pub state: String,
    pub lifetime: String,
    pub blocking: bool,
    pub parent_operation: Option<String>,
    pub child_operation: Option<String>,
    pub conversation: Option<String>,
    pub usage: Usage,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub scope: Scope,
    pub status: Status,
    pub position: Option<u64>,
    pub sessions: Vec<Entry>,
    pub rows: Vec<Row>,
    pub work: Vec<Work>,
    pub unavailable: BTreeMap<String, Fault>,
}
pub struct Overview {
    observation: Observation,
    client: Client,
    connection: tokio::sync::watch::Receiver<crate::driver::Status>,
    refresh_fault: Option<Fault>,
}
impl Overview {
    pub async fn open(client: &Client) -> Result<Self, Fault> {
        let interface = Interface::load(client, client.welcome().scope).await?;
        let mut members = BTreeMap::from([(
            "sessions".into(),
            interface.query(misa_proto::directory::SESSIONS, vec![])?,
        )]);
        for (name, id) in [("overview", "daemon.overview"), ("work", "daemon.work")] {
            if interface.queries.contains_key(id) {
                let mut member = interface.query(id, vec![])?;
                member.optional = true;
                members.insert(name.into(), member);
            }
        }
        Ok(Self {
            client: client.clone(),
            connection: client.status(),
            refresh_fault: None,
            observation: client
                .observe(
                    Selection {
                        scope: interface.scope,
                        members,
                    },
                    None,
                )
                .await?,
        })
    }
    pub fn watch(&self) -> tokio::sync::watch::Receiver<u64> {
        self.observation.watch()
    }
    /// Follow authenticated daemon incarnations. A changed greeting first makes
    /// old data stale; the next wait rebuilds against the new owner's catalog.
    pub async fn changed(&mut self) -> Result<(), Fault> {
        if matches!(
            self.connection.borrow().phase,
            crate::driver::Phase::Disconnected
        ) {
            return Err(Fault::new("closed", "Daemon relationship disconnected"));
        }
        let scope = self
            .observation
            .inspect(|replica, _| replica.selection().scope.clone());
        if scope.as_ref() != Some(&self.client.welcome().scope) {
            if self.refresh_fault.is_some() {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
            match Self::open(&self.client).await {
                Ok(replacement) => {
                    let current = replacement
                        .observation
                        .inspect(|replica, _| replica.selection().scope.clone());
                    if current.as_ref() == Some(&self.client.welcome().scope) {
                        *self = replacement;
                    }
                }
                Err(fault) => self.refresh_fault = Some(fault),
            }
            return Ok(());
        }
        tokio::select! {
            changed = self.observation.changed() => changed,
            changed = self.connection.changed() => changed.map_err(|_|Fault::new("closed","Daemon relationship closed")),
        }
    }
    pub fn snapshot(&self) -> Result<Snapshot, Fault> {
        self.observation
            .inspect(|replica, _| {
                let mut result = Snapshot {
                    scope: replica.selection().scope.clone(),
                    status: replica.status().clone(),
                    position: replica.position(),
                    sessions: vec![],
                    rows: vec![],
                    work: vec![],
                    unavailable: BTreeMap::new(),
                };
                if result.scope != self.client.welcome().scope {
                    result.status =
                        Status::Stale(self.refresh_fault.clone().unwrap_or_else(|| {
                            Fault::new("owner_changed", "Daemon restarted; refreshing overview")
                        }));
                }
                let Some(members) = replica.last_good() else {
                    return Ok(result);
                };
                for (name, member) in members {
                    match member {
                        MemberState::Unavailable(fault) => {
                            result.unavailable.insert(name.clone(), fault.clone());
                        }
                        MemberState::Value(value) => match name.as_str() {
                            "sessions" => result.sessions = misa_proto::directory::entries(value)?,
                            "overview" => match interface::decode(value) {
                                Ok(rows) => result.rows = rows,
                                Err(fault) => {
                                    result.unavailable.insert(name.clone(), fault);
                                }
                            },
                            "work" => match interface::decode(value) {
                                Ok(work) => result.work = work,
                                Err(fault) => {
                                    result.unavailable.insert(name.clone(), fault);
                                }
                            },
                            _ => {}
                        },
                        _ => return Err(Fault::query("Overview member must be data")),
                    }
                }
                Ok(result)
            })
            .unwrap_or_else(|| Err(Fault::new("closed", "Overview observation closed")))
    }
}
