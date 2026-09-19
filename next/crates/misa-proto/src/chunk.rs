//! Session messages in bounded chunks. The high header bit means another chunk follows;
//! the low bits name only the bytes in this chunk. No total length is advertised or
//! allocated up front. A message becomes visible only after its final chunk arrives.
//!
//! Blob traffic keeps its separate, size-limited framing. Session snapshots can grow
//! with the log, so their limit belongs to each chunk, not to the canonical document.
use std::collections::VecDeque;
use std::io::{self, Write};

use serde::Serialize;

use crate::frame::FrameError;
pub use crate::frame::decode;

pub const MAX_CHUNK: usize = 64 * 1024;
const MORE: u32 = 1 << 31;

/// Count framed bytes without allocating the payload, for transport admission.
pub fn encoded_size_with_limit<T: Serialize>(value: &T, limit: usize) -> Result<usize, FrameError> {
    struct Count {
        bytes: usize,
        limit: usize,
    }
    impl Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.bytes) {
                return Err(io::Error::other("Logical message exceeds byte limit"));
            }
            self.bytes += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count { bytes: 0, limit };
    ciborium::ser::into_writer(value, &mut count)
        .map_err(|error| FrameError::Codec(error.to_string()))?;
    count
        .bytes
        .checked_add(count.bytes.div_ceil(MAX_CHUNK).max(1) * 4)
        .ok_or_else(|| FrameError::Codec("Logical message size overflow".into()))
}

/// Serialize directly into framed chunks, without first making a second whole payload.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, FrameError> {
    encode_with_limit(value, usize::MAX)
}

/// Bound total logical serialization as well as each wire chunk.
pub fn encode_with_limit<T: Serialize>(value: &T, limit: usize) -> Result<Vec<u8>, FrameError> {
    let mut writer = Encoder {
        bytes: vec![0; 4],
        header: 0,
        length: 0,
        limit,
        written: 0,
    };
    ciborium::ser::into_writer(value, &mut writer)
        .map_err(|error| FrameError::Codec(error.to_string()))?;
    writer.finish(false);
    Ok(writer.bytes)
}

struct Encoder {
    bytes: Vec<u8>,
    header: usize,
    length: usize,
    limit: usize,
    written: usize,
}
impl Encoder {
    fn finish(&mut self, more: bool) {
        let header = self.length as u32 | if more { MORE } else { 0 };
        self.bytes[self.header..self.header + 4].copy_from_slice(&header.to_be_bytes());
    }
}
impl Write for Encoder {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let written = bytes.len();
        if written > self.limit.saturating_sub(self.written) {
            return Err(io::Error::other("Logical message exceeds byte limit"));
        }
        self.written += written;
        while !bytes.is_empty() {
            if self.length == MAX_CHUNK {
                self.finish(true);
                self.header = self.bytes.len();
                self.bytes.extend_from_slice(&[0; 4]);
                self.length = 0;
            }
            let count = bytes.len().min(MAX_CHUNK - self.length);
            self.bytes.extend_from_slice(&bytes[..count]);
            self.length += count;
            bytes = &bytes[count..];
        }
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct Decoder {
    header: [u8; 4],
    header_len: usize,
    remaining: usize,
    more: bool,
    message: Vec<u8>,
    ready: VecDeque<Vec<u8>>,
    failed: bool,
}
impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume arbitrary read boundaries. Memory grows only for bytes received, never
    /// for a peer's claimed length. An invalid header permanently poisons the decoder.
    pub fn push(&mut self, mut bytes: &[u8]) -> Result<(), FrameError> {
        if self.failed {
            return Err(FrameError::Codec(
                "decoder is closed after invalid framing".into(),
            ));
        }
        while !bytes.is_empty() {
            if self.header_len < 4 {
                let count = bytes.len().min(4 - self.header_len);
                self.header[self.header_len..self.header_len + count]
                    .copy_from_slice(&bytes[..count]);
                self.header_len += count;
                bytes = &bytes[count..];
                if self.header_len < 4 {
                    break;
                }
                let header = u32::from_be_bytes(self.header);
                self.more = header & MORE != 0;
                self.remaining = (header & !MORE) as usize;
                if self.remaining == 0 || self.remaining > MAX_CHUNK {
                    self.failed = true;
                    return Err(if self.remaining == 0 {
                        FrameError::Empty
                    } else {
                        FrameError::TooLarge(self.remaining, MAX_CHUNK)
                    });
                }
            }
            let count = bytes.len().min(self.remaining);
            self.message.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            self.remaining -= count;
            if self.remaining == 0 {
                self.header_len = 0;
                if !self.more {
                    self.ready.push_back(std::mem::take(&mut self.message));
                }
            }
        }
        Ok(())
    }
    pub fn next(&mut self) -> Option<Result<Vec<u8>, FrameError>> {
        self.ready.pop_front().map(Ok)
    }
    pub fn is_empty(&self) -> bool {
        self.header_len == 0 && self.message.is_empty() && self.ready.is_empty()
    }
    /// Check EOF, so the caller can distinguish a closed stream from a partial message.
    pub fn finish(&self) -> Result<(), FrameError> {
        if self.header_len != 0 || !self.message.is_empty() {
            Err(FrameError::Truncated(
                self.message.len(),
                self.message.len() + self.remaining.max(1),
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_canonical_value_can_exceed_the_old_message_limit() {
        let value = "x".repeat(crate::MAX_CONTROL_FRAME + 1024);
        let bytes = encode(&value).unwrap();
        let mut decoder = Decoder::new();
        for read in bytes.chunks(7919) {
            decoder.push(read).unwrap();
        }
        let payload = decoder.next().unwrap().unwrap();
        assert_eq!(decode::<String>(&payload).unwrap(), value);
        assert!(decoder.is_empty());
        decoder.finish().unwrap();
    }

    #[test]
    fn every_read_boundary_preserves_message_atomicity() {
        let value = "a value";
        let bytes = encode(&value).unwrap();
        for split in 0..bytes.len() {
            let mut decoder = Decoder::new();
            decoder.push(&bytes[..split]).unwrap();
            assert!(decoder.next().is_none());
            decoder.push(&bytes[split..]).unwrap();
            assert_eq!(
                decode::<String>(&decoder.next().unwrap().unwrap()).unwrap(),
                value
            );
        }
        let bytes = encode(&"x".repeat(MAX_CHUNK * 2)).unwrap();
        let mut decoder = Decoder::new();
        decoder.push(&bytes[..MAX_CHUNK + 4]).unwrap();
        assert!(decoder.next().is_none());
        assert!(decoder.finish().is_err());
        decoder.push(&bytes[MAX_CHUNK + 4..]).unwrap();
        assert!(decoder.next().is_some());
    }

    #[test]
    fn consecutive_messages_in_one_read_stay_separate() {
        let bytes: Vec<u8> = (0..3).flat_map(|n| encode(&n).unwrap()).collect();
        let mut decoder = Decoder::new();
        decoder.push(&bytes).unwrap();
        for n in 0..3 {
            assert_eq!(decode::<i32>(&decoder.next().unwrap().unwrap()).unwrap(), n);
        }
        assert!(decoder.is_empty());
    }

    #[test]
    fn a_bad_chunk_is_refused_before_allocating_its_payload() {
        for header in [0, MORE, MAX_CHUNK as u32 + 1, u32::MAX] {
            let mut decoder = Decoder::new();
            assert!(decoder.push(&header.to_be_bytes()).is_err());
            assert_eq!(decoder.message.capacity(), 0);
            assert!(decoder.push(&[0]).is_err());
        }
    }

    #[test]
    fn the_writer_never_exceeds_the_chunk_bound() {
        for length in [MAX_CHUNK - 5, MAX_CHUNK, MAX_CHUNK * 3] {
            let bytes = encode(&"x".repeat(length)).unwrap();
            let mut offset = 0;
            loop {
                let header = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap());
                let count = (header & !MORE) as usize;
                assert!((1..=MAX_CHUNK).contains(&count));
                offset += 4 + count;
                if header & MORE == 0 {
                    break;
                }
            }
            assert_eq!(offset, bytes.len());
        }
    }
}
