//! Bounded, reserved control replies. Broadcast lag cannot discard a requested file.
use std::collections::BTreeMap;
use std::sync::Mutex;
use tokio::sync::mpsc;
use crate::Emission;

const CAPACITY: usize = 64;
#[derive(Default)]
pub(crate) struct Replies(Mutex<BTreeMap<u64, Connection>>);
struct Connection {
    sender: mpsc::Sender<Emission>,
    pending: BTreeMap<u64, mpsc::OwnedPermit<Emission>>,
}
impl Replies {
    pub fn register(&self, recipient: u64) -> mpsc::Receiver<Emission> {
        let (sender, receiver) = mpsc::channel(CAPACITY);
        self.0.lock().unwrap().insert(recipient, Connection {sender, pending: BTreeMap::new()});
        receiver
    }
    pub fn remove(&self, recipient: u64) { self.0.lock().unwrap().remove(&recipient); }
    pub fn reserve(&self, recipient: u64, id: u64) -> Result<(), String> {
        let mut connections = self.0.lock().unwrap();
        let connection = connections.get_mut(&recipient).ok_or("The requesting connection has closed")?;
        if connection.pending.contains_key(&id) { return Err("This request ID is already pending".into()); }
        let permit = connection.sender.clone().try_reserve_owned()
            .map_err(|_| "Too many unanswered save requests on this connection")?;
        connection.pending.insert(id, permit);
        Ok(())
    }
    pub fn cancel(&self, recipient: u64, id: u64) {
        if let Some(connection) = self.0.lock().unwrap().get_mut(&recipient) { connection.pending.remove(&id); }
    }
    pub fn send(&self, recipient: u64, id: u64, emission: Emission) {
        if let Some(permit) = self.0.lock().unwrap().get_mut(&recipient).and_then(|connection| connection.pending.remove(&id)) {
            permit.send(emission);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn reserved_replies_survive_full_output_and_bound_inflight_work() {
        let replies = Replies::default(); let mut receiver = replies.register(1);
        for id in 0..CAPACITY as u64 { replies.reserve(1, id).unwrap(); }
        assert!(replies.reserve(1, 64).is_err());
        for id in 0..CAPACITY as u64 {
            replies.send(1, id, Emission {recipient: Some(1), seq: id,
                event: misa_proto::SessionEvent::Status {text: id.to_string()}});
        }
        assert!(replies.reserve(1, 64).is_err());
        for id in 0..CAPACITY as u64 { assert_eq!(receiver.recv().await.unwrap().seq, id); }
        replies.reserve(1, 64).unwrap(); replies.cancel(1, 64);
        replies.remove(1);
        assert!(replies.reserve(1, 65).is_err());
        assert!(receiver.recv().await.is_none());
    }
}
