//! Length-delimited framing for a byte stream.
//!
//! The same shape the previous system used at its effect boundary and this
//! repository's sibling projects use on the wire: a four-byte big-endian length,
//! then a CBOR payload. The length is checked *before* any payload buffer is
//! allocated, so a peer cannot ask for a gigabyte by saying so.
//!
//! CBOR rather than a schema-fixed binary: it is self-describing, it keeps its
//! field names, and a reader can therefore ignore a field it does not know. Two
//! ends of different minor versions stay compatible without a version negotiation
//! protocol for every change.

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

/// Bytes of length prefix.
pub const HEADER_BYTES: usize = 4;

/// The default ceiling for one message.
pub const MAX_FRAME: usize = crate::MAX_CONTROL_FRAME;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("a frame of {0} bytes exceeds the {1} byte limit")]
    TooLarge(usize, usize),
    #[error("a frame has no payload")]
    Empty,
    #[error("a frame is truncated: {0} of {1} bytes arrived")]
    Truncated(usize, usize),
    #[error("a frame is not valid cbor: {0}")]
    Codec(String),
    #[error("a peer sent {0} bytes without completing a frame")]
    StreamOverrun(usize),
}

/// Frame one message.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, FrameError> {
    encode_within(value, MAX_FRAME)
}

/// Frame one message under a caller's own ceiling.
pub fn encode_within<T: Serialize>(value: &T, limit: usize) -> Result<Vec<u8>, FrameError> {
    let mut payload = Vec::new();
    ciborium::ser::into_writer(value, &mut payload).map_err(|err| FrameError::Codec(err.to_string()))?;
    if payload.is_empty() {
        return Err(FrameError::Empty);
    }
    if payload.len() > limit {
        return Err(FrameError::TooLarge(payload.len(), limit));
    }
    let mut frame = Vec::with_capacity(HEADER_BYTES + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Read one message out of a frame's payload.
pub fn decode<T: DeserializeOwned>(payload: &[u8]) -> Result<T, FrameError> {
    if payload.is_empty() {
        return Err(FrameError::Empty);
    }
    ciborium::de::from_reader(payload).map_err(|err| FrameError::Codec(err.to_string()))
}

/// Reassembles framed messages from arbitrarily chopped bytes.
///
/// A QUIC stream hands over whatever arrived, so a reader is a buffer and a
/// decision about when the next message is complete — not an async read of a
/// known length. Keeping that decision here means it is tested once, in a test
/// that never touches a socket.
#[derive(Debug, Default)]
pub struct Decoder {
    buffer: Vec<u8>,
    limit: usize,
}

impl Decoder {
    pub fn new() -> Self {
        Decoder { buffer: Vec::new(), limit: MAX_FRAME }
    }

    pub fn with_limit(limit: usize) -> Self {
        Decoder { buffer: Vec::new(), limit }
    }

    /// Bytes held while a frame is incomplete.
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }

    /// Add bytes. Refuses once an incomplete frame has already exceeded the limit,
    /// so a peer cannot make the reader grow without bound.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), FrameError> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > self.limit + HEADER_BYTES {
            return Err(FrameError::StreamOverrun(self.buffer.len()));
        }
        Ok(())
    }

    /// The next frame's payload, if a whole one has arrived.
    ///
    /// A returned payload is always non-empty and within the limit; the length
    /// prefix is consumed either way, so a bad prefix ends the stream rather than
    /// being retried forever.
    pub fn next(&mut self) -> Option<Result<Vec<u8>, FrameError>> {
        if self.buffer.len() < HEADER_BYTES {
            return None;
        }
        let length = u32::from_be_bytes([self.buffer[0], self.buffer[1], self.buffer[2], self.buffer[3]]) as usize;
        if length == 0 {
            self.buffer.drain(..HEADER_BYTES);
            return Some(Err(FrameError::Empty));
        }
        if length > self.limit {
            self.buffer.clear();
            return Some(Err(FrameError::TooLarge(length, self.limit)));
        }
        if self.buffer.len() < HEADER_BYTES + length {
            return None;
        }
        let payload = self.buffer[HEADER_BYTES..HEADER_BYTES + length].to_vec();
        self.buffer.drain(..HEADER_BYTES + length);
        Some(Ok(payload))
    }

    /// True when no bytes are held.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{ClientMsg, ClientInfo, Query, SubId};

    fn hello() -> ClientMsg {
        ClientMsg::Hello {
            version: crate::PROTOCOL_VERSION,
            client: ClientInfo::new("test", "0.1.0"),
        }
    }

    #[test]
    fn a_message_round_trips_through_a_frame() {
        let frame = encode(&hello()).unwrap();
        assert!(frame.len() > HEADER_BYTES);
        let payload = &frame[HEADER_BYTES..];
        assert_eq!(decode::<ClientMsg>(payload).unwrap(), hello());
    }

    #[test]
    fn a_decoder_reassembles_a_message_split_at_every_boundary() {
        let frame = encode(&ClientMsg::Subscribe { id: SubId(1), query: Query::new("session.view") }).unwrap();
        // Every byte boundary except the last leaves an incomplete frame, and the
        // next push completes it from whatever arrived.
        for split in 0..frame.len() {
            let mut decoder = Decoder::new();
            decoder.push(&frame[..split]).unwrap();
            assert!(decoder.next().is_none(), "a partial frame was reported complete at {split}");
            decoder.push(&frame[split..]).unwrap();
            let payload = decoder.next().expect("a complete frame").unwrap();
            assert!(matches!(decode::<ClientMsg>(&payload).unwrap(), ClientMsg::Subscribe { .. }));
            assert!(decoder.next().is_none());
            assert!(decoder.is_empty());
        }
    }

    #[test]
    fn a_decoder_returns_several_frames_from_one_read() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode(&hello()).unwrap());
        bytes.extend_from_slice(&encode(&ClientMsg::Ping { nonce: 1 }).unwrap());
        bytes.extend_from_slice(&encode(&ClientMsg::Ping { nonce: 2 }).unwrap());
        let mut decoder = Decoder::new();
        decoder.push(&bytes).unwrap();
        let first = decoder.next().unwrap().unwrap();
        assert!(matches!(decode::<ClientMsg>(&first).unwrap(), ClientMsg::Hello { .. }));
        let second = decoder.next().unwrap().unwrap();
        assert!(matches!(decode::<ClientMsg>(&second).unwrap(), ClientMsg::Ping { nonce: 1 }));
        let third = decoder.next().unwrap().unwrap();
        assert!(matches!(decode::<ClientMsg>(&third).unwrap(), ClientMsg::Ping { nonce: 2 }));
        assert!(decoder.next().is_none());
    }

    #[test]
    fn a_declared_length_over_the_limit_is_refused_before_the_payload() {
        let mut decoder = Decoder::with_limit(16);
        decoder.push(&64u32.to_be_bytes()).unwrap();
        let error = decoder.next().unwrap().unwrap_err();
        assert_eq!(error, FrameError::TooLarge(64, 16));
        // A refused frame is consumed, not retried forever.
        assert!(decoder.next().is_none());
        assert!(decoder.is_empty());
    }

    #[test]
    fn a_zero_length_frame_is_refused() {
        let mut decoder = Decoder::new();
        decoder.push(&0u32.to_be_bytes()).unwrap();
        assert_eq!(decoder.next().unwrap().unwrap_err(), FrameError::Empty);
    }

    #[test]
    fn a_peer_cannot_grow_the_buffer_without_bound() {
        let mut decoder = Decoder::with_limit(16);
        decoder.push(&16u32.to_be_bytes()).unwrap();
        assert_eq!(decoder.push(&[0u8; 32]), Err(FrameError::StreamOverrun(36)));
    }

    #[test]
    fn an_oversized_message_is_refused_by_the_writer() {
        let payload = "x".repeat(4096);
        match encode_within(&ClientMsg::Ping { nonce: 1 }, 8) {
            Err(FrameError::TooLarge(_, 8)) => {}
            other => panic!("expected a refusal, got {other:?}"),
        }
        // The limit is on the payload, not on the frame.
        assert!(encode_within(&payload, payload.len() + 16).is_ok());
    }
}
