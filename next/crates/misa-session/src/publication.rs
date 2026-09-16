//! The owner's publication clock and bounded transient replay.
//!
//! Access is serialized by the same lock as domain state and canonical content.
//! Canonical history lives in `Canonical`; token traffic cannot evict it.
use std::collections::VecDeque;

use misa_proto::sync::StreamUpdate;
use tokio::sync::watch;

const LIVE_REPLAY_BYTES: usize = 1024 * 1024;
const LIVE_REPLAY_BATCHES: usize = 512;

struct Batch {
    position: u64,
    bytes: usize,
    streams: Vec<StreamUpdate>,
}

pub(crate) struct Publications {
    position: u64,
    live_floor: u64,
    live_bytes: usize,
    live: VecDeque<Batch>,
    changed: watch::Sender<u64>,
}

impl Default for Publications {
    fn default() -> Self {
        let (changed, _) = watch::channel(0);
        Self {
            position: 0,
            live_floor: 0,
            live_bytes: 0,
            live: VecDeque::new(),
            changed,
        }
    }
}

impl Publications {
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Subscribe while holding the owner lock, before capturing the initial read.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Called after all parts of an owner transition have been applied, while
    /// still holding its lock. Readers awakened here cannot see half a commit.
    pub fn commit(&mut self, streams: Vec<StreamUpdate>) {
        self.position = self
            .position
            .checked_add(1)
            .expect("owner publication clock exhausted");
        if !streams.is_empty() {
            let bytes = streams.iter().map(stream_bytes).sum();
            self.live_bytes += bytes;
            self.live.push_back(Batch {
                position: self.position,
                bytes,
                streams,
            });
            while self.live.len() > LIVE_REPLAY_BATCHES || self.live_bytes > LIVE_REPLAY_BYTES {
                let removed = self
                    .live
                    .pop_front()
                    .expect("retained bytes belong to a batch");
                self.live_bytes -= removed.bytes;
                self.live_floor = removed.position;
            }
        }
        self.changed.send_replace(self.position);
    }

    /// Missing transient history requires a live reset, independently of whether
    /// canonical changes are still retained. Never return a partial append chain.
    pub fn streams_since(&self, position: u64) -> Option<Vec<StreamUpdate>> {
        if position < self.live_floor || position > self.position {
            return None;
        }
        Some(
            self.live
                .iter()
                .filter(|batch| batch.position > position)
                .flat_map(|batch| batch.streams.iter().cloned())
                .collect(),
        )
    }
}

fn stream_bytes(update: &StreamUpdate) -> usize {
    // Include identity/record overhead so empty chunks cannot evade the budget.
    std::mem::size_of::<StreamUpdate>()
        + match update {
            StreamUpdate::Current { stream } => {
                stream.id.len() + stream.role.len() + stream.text.len()
            }
            StreamUpdate::Append { id, text, .. } => id.len() + text.len(),
            StreamUpdate::End { id } => id.len(),
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append(offset: usize, text: String) -> StreamUpdate {
        StreamUpdate::Append {
            id: "attempt.text".into(),
            offset,
            text,
        }
    }

    #[test]
    fn state_and_stream_transitions_share_one_watch_boundary() {
        let mut publications = Publications::default();
        let mut watch = publications.subscribe();
        publications.commit(vec![append(0, "é".into())]);
        publications.commit(vec![]);
        publications.commit(vec![StreamUpdate::End {
            id: "attempt.text".into(),
        }]);
        assert_eq!(*watch.borrow_and_update(), 3);
        assert_eq!(publications.streams_since(0).unwrap().len(), 2);
        assert_eq!(publications.streams_since(2).unwrap().len(), 1);
        assert!(publications.streams_since(3).unwrap().is_empty());
        assert!(publications.streams_since(4).is_none());
    }

    #[test]
    fn evicted_transient_history_requires_a_reset() {
        let mut publications = Publications::default();
        for offset in 0..LIVE_REPLAY_BATCHES + 1 {
            publications.commit(vec![append(offset, "x".into())]);
        }
        assert!(publications.streams_since(0).is_none());
        assert_eq!(
            publications.streams_since(1).unwrap().len(),
            LIVE_REPLAY_BATCHES
        );
        // Arbitrarily many domain publications do not evict retained live text.
        for _ in 0..1000 {
            publications.commit(vec![]);
        }
        assert_eq!(
            publications.streams_since(1).unwrap().len(),
            LIVE_REPLAY_BATCHES
        );
    }

    #[test]
    fn oversized_batch_is_not_partially_retained() {
        let mut publications = Publications::default();
        publications.commit(vec![append(0, "x".repeat(LIVE_REPLAY_BYTES + 1))]);
        assert!(publications.streams_since(0).is_none());
        assert!(publications.streams_since(1).unwrap().is_empty());
        assert_eq!(publications.live_bytes, 0);
    }
}
