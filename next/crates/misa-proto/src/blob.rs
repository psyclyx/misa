//! Bulk content, on its own connection.
//!
//! A view node that carries an image names a blob rather than inlining bytes, so a
//! transcript stays small and a client fetches only what it can actually display. That
//! decision is what makes this module necessary, and it is also what shapes it: the
//! vocabulary here is three messages, and every one of them is about *bytes by hash*.
//!
//! # Why not the session connection
//!
//! Because the two have nothing to do with each other. The session connection is small,
//! frequent, ordered control traffic with a state machine behind it; a blob is up to
//! [`MAX_BLOB_BYTES`] of opaque content with no state machine at all. Putting them on one
//! connection would mean a 4 MiB image queued behind — or ahead of — a token stream, and
//! [`crate::MAX_CONTROL_FRAME`] would have to be raised for every message so that images
//! could be sent, which is exactly the bound that keeps a hostile peer from making a
//! client allocate.
//!
//! # Why a hash is enough
//!
//! A blob is named by the hash of its content, so a name cannot be a path: a client
//! cannot ask for `/etc/shadow` by calling it one, and it cannot be handed a different
//! file than the one it asked for. That is the property that makes it safe to serve the
//! same store to every admitted client, and it is why the server needs no per-blob
//! permission table.

use serde::{Deserialize, Serialize};

use crate::Fault;

/// The largest blob either end will move in one message.
///
/// The kernel's content-addressed store is bounded by this same number, so a blob that
/// exists can always be served and one that is too large to move is refused where it is
/// stored rather than when somebody tries to fetch it.
pub const MAX_BLOB_BYTES: usize = 32 * 1024 * 1024;

/// The ceiling for one framed message on the blob connection.
///
/// A blob plus the CBOR envelope around it: the hash, the media type, and the four-byte
/// length prefix. Stated as its own number because the reader uses it to refuse a
/// declared length *before* allocating anything.
pub const MAX_BLOB_FRAME: usize = MAX_BLOB_BYTES + 8192;

/// What a client asks a server for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "blob", rename_all = "snake_case")]
pub enum BlobMsg {
    /// One blob, by content hash.
    Get { hash: String },
    /// Which of these hashes the server already has.
    ///
    /// Asked before an upload, because content addressing makes the answer useful: bytes
    /// the server already holds are bytes that do not need to be sent, and a client
    /// that has attached the same screenshot before sends nothing.
    Have { hashes: Vec<String> },
    /// Bytes, to be named by content.
    Put {
        bytes: Vec<u8>,
        /// What the bytes are, when the client knows better than magic numbers do.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<String>,
    },
}

impl BlobMsg {
    /// A one-word name for diagnostics.
    pub fn name(&self) -> &'static str {
        match self {
            BlobMsg::Get { .. } => "get",
            BlobMsg::Have { .. } => "have",
            BlobMsg::Put { .. } => "put",
        }
    }

    /// Whether this is a message a server should carry out.
    ///
    /// Checked before a store is touched, so a peer cannot make it walk a path that is not
    /// a hash or try to hold more bytes than the bound allows. The size bound is the same
    /// number the store uses, which is why it is stated once, here.
    pub fn acceptable(&self) -> Result<(), Fault> {
        let bad = |hash: &str| Fault::new("blob.hash", format!("`{hash}` is not a content hash"));
        match self {
            BlobMsg::Get { hash } => valid_hash(hash).then_some(()).ok_or_else(|| bad(hash)),
            BlobMsg::Have { hashes } => {
                match hashes.iter().find(|hash| !valid_hash(hash)) {
                    Some(hash) => Err(bad(hash)),
                    None => Ok(()),
                }
            }
            BlobMsg::Put { bytes, .. } if bytes.len() > MAX_BLOB_BYTES => Err(Fault::new(
                "blob.size",
                format!("{} bytes is larger than the {MAX_BLOB_BYTES} byte bound", bytes.len()),
            )),
            BlobMsg::Put { .. } => Ok(()),
        }
    }
}

/// What a server answers with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum BlobReply {
    /// The bytes, and what they are.
    ///
    /// The media type travels with them because the server sniffed the magic numbers when
    /// they were stored, and a client that has to guess would guess wrong for exactly the
    /// files worth displaying.
    Bytes {
        hash: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<String>,
        bytes: Vec<u8>,
    },
    /// Bytes that were stored, named by content.
    Stored { hash: String, len: u64, media: Option<String> },
    /// Which of the asked-for hashes the server holds.
    Have { hashes: Vec<String> },
    /// The server does not hold it. Not an error: a blob a client asked for may have
    /// been dropped, and the answer is a fact about the store.
    Missing { hash: String },
    /// The request was refused. Bytes are never partially served: a client gets the
    /// whole blob or a reason.
    Refused { fault: Fault },
}

impl BlobReply {
    /// A one-word name for diagnostics.
    pub fn name(&self) -> &'static str {
        match self {
            BlobReply::Bytes { .. } => "bytes",
            BlobReply::Stored { .. } => "stored",
            BlobReply::Have { .. } => "have",
            BlobReply::Missing { .. } => "missing",
            BlobReply::Refused { .. } => "refused",
        }
    }

    /// The refusal a server sends for a request it will not serve.
    pub fn refused(message: impl Into<String>) -> BlobReply {
        BlobReply::Refused { fault: Fault::new("blob.refused", message) }
    }

    /// A refusal for a hash that is not one.
    ///
    /// Checked on both sides: a hash is about to become a filename, and "it is 64
    /// lowercase hex characters" is cheaper to check here than to defend everywhere it
    /// could arrive.
    pub fn bad_hash(hash: &str) -> BlobReply {
        BlobReply::Refused {
            fault: Fault::new("blob.hash", format!("`{hash}` is not a content hash")),
        }
    }
}

/// Whether a string is one of this system's content hashes.
///
/// Lowercase hex, 64 characters, and nothing else. An uppercase hash is not a hash this
/// system produced, so it is refused rather than normalised: a name that two ends
/// disagree about is a name that will be mis-served.
pub fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T: Serialize + for<'de> Deserialize<'de>>(value: &T) -> T {
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(value, &mut bytes).unwrap();
        ciborium::de::from_reader(&bytes[..]).unwrap()
    }

    #[test]
    fn every_blob_message_round_trips() {
        let messages = [
            BlobMsg::Get { hash: "a".repeat(64) },
            BlobMsg::Have { hashes: vec!["b".repeat(64)] },
            BlobMsg::Put { bytes: vec![0x89, b'P', b'N', b'G'], media: Some("image/png".into()) },
            BlobMsg::Put { bytes: b"text".to_vec(), media: None },
        ];
        for message in messages {
            assert_eq!(round_trip(&message), message);
        }
    }

    #[test]
    fn every_blob_reply_round_trips() {
        let replies = [
            BlobReply::Bytes {
                hash: "c".repeat(64),
                media: Some("image/jpeg".into()),
                bytes: vec![0xff, 0xd8, 0xff],
            },
            BlobReply::Stored { hash: "d".repeat(64), len: 3, media: None },
            BlobReply::Have { hashes: Vec::new() },
            BlobReply::Missing { hash: "e".repeat(64) },
            BlobReply::refused("too large"),
        ];
        for reply in replies {
            assert_eq!(round_trip(&reply), reply);
        }
    }

    #[test]
    fn a_name_that_is_not_a_hash_is_refused_rather_than_normalised() {
        assert!(valid_hash(&"0123456789abcdef".repeat(4)));
        assert!(!valid_hash(&"0123456789abcdef".repeat(4).to_uppercase()));
        assert!(!valid_hash("../../etc/shadow"));
        assert!(!valid_hash(&"a".repeat(63)));
        assert!(!valid_hash(&"a".repeat(65)));
        assert!(matches!(BlobReply::bad_hash("nope"), BlobReply::Refused { .. }));
    }

    #[test]
    fn a_largest_blob_and_its_envelope_fit_in_one_frame() {
        // A blob at the bound has to fit in a frame, or the store would hold something
        // that could never be served.
        assert!(MAX_BLOB_FRAME > MAX_BLOB_BYTES + crate::frame::HEADER_BYTES);
        let payload = vec![0u8; 4096];
        let reply = BlobReply::Bytes { hash: "f".repeat(64), media: Some("image/png".into()), bytes: payload };
        let frame = crate::frame::encode_within(&reply, MAX_BLOB_FRAME).expect("a blob frames");
        // The envelope is small next to the bytes, which is the whole claim.
        assert!(frame.len() < 4096 + 512, "the envelope is {} bytes", frame.len() - 4096);
    }

    #[test]
    fn a_message_a_server_should_not_carry_out_is_refused_before_a_store_sees_it() {
        // A name that is not a hash would become a filename, so it is refused here rather
        // than defended against in every store.
        let bad = BlobMsg::Get { hash: "../../etc/shadow".into() };
        assert_eq!(bad.acceptable().unwrap_err().code, "blob.hash");
        assert!(BlobMsg::Have { hashes: vec!["a".repeat(64), "no".into()] }.acceptable().is_err());
        assert!(BlobMsg::Have { hashes: vec!["a".repeat(64)] }.acceptable().is_ok());
        // And bytes beyond the bound are refused for the same reason: the store's bound and
        // the wire's are the same number, so the answer here is final.
        let oversized = BlobMsg::Put { bytes: vec![0u8; MAX_BLOB_BYTES + 1], media: None };
        assert_eq!(oversized.acceptable().unwrap_err().code, "blob.size");
        assert!(BlobMsg::Put { bytes: vec![0u8; 8], media: None }.acceptable().is_ok());
        assert!(BlobMsg::Get { hash: "a".repeat(64) }.acceptable().is_ok());
    }

    #[test]
    fn a_message_that_names_itself_says_something_useful_in_a_log() {
        assert_eq!(BlobMsg::Get { hash: String::new() }.name(), "get");
        assert_eq!(BlobReply::Have { hashes: Vec::new() }.name(), "have");
    }
}
