//! Human-portable daemon addresses and pairing invitations.
use serde::{Deserialize, Serialize};

/// How to reach a session.
///
/// Carried as text so a person can copy one into a chat message. The node part is
/// an endpoint address in whatever form the transport understands; this crate
/// neither parses nor validates it, because it has no opinion about transports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub node: String,
    pub session: String,
}
impl Ticket {
    pub fn new(node: impl Into<String>, session: impl Into<String>) -> Self {
        Ticket {
            node: node.into(),
            session: session.into(),
        }
    }
}

impl std::fmt::Display for Ticket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "misa:{}:{}", self.node, self.session)
    }
}

/// A ticket and the code that pairs it, as one string.
///
/// The only place this is ever carried by hand is a camera pointed at a terminal, so it is one
/// line and it is unambiguous: `misa-pair:` then a ticket, then `#`, then the code. A `#` cannot
/// appear in a ticket — a node is an endpoint id and an address, a session is a name — so the
/// two halves can never be confused for each other, however an address is spelled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pairing {
    pub ticket: Ticket,
    pub code: String,
}

impl Pairing {
    pub fn new(ticket: Ticket, code: impl Into<String>) -> Pairing {
        Pairing {
            ticket,
            code: code.into(),
        }
    }
}

impl Pairing {
    /// What a client was handed: a ticket, and the code that pairs it when there is one.
    ///
    /// One entry point for both shapes, because a person copies one string off a screen — the
    /// ticket a daemon printed, or the pairing line beside its QR code — and a client that made
    /// them find the code separately would be a client with a second thing to get wrong.
    pub fn given(text: &str) -> Result<(Ticket, Option<String>), String> {
        let text = text.trim();
        if text.starts_with("misa-pair:") {
            let pairing: Pairing = text.parse()?;
            return Ok((pairing.ticket, Some(pairing.code)));
        }
        Ok((text.parse::<Ticket>()?, None))
    }
}

impl std::fmt::Display for Pairing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "misa-pair:{}#{}", self.ticket, self.code)
    }
}

impl std::str::FromStr for Pairing {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let rest = text
            .trim()
            .strip_prefix("misa-pair:")
            .ok_or("a pairing string starts with `misa-pair:`")?;
        let (ticket, code) = rest
            .rsplit_once('#')
            .ok_or("a pairing string is `misa-pair:<ticket>#<code>`")?;
        if code.trim().is_empty() {
            return Err("a pairing string needs a code".into());
        }
        Ok(Pairing::new(ticket.parse()?, code.trim()))
    }
}

impl std::str::FromStr for Ticket {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let rest = text
            .strip_prefix("misa:")
            .ok_or("a ticket starts with `misa:`")?;
        let (node, session) = rest
            .rsplit_once(':')
            .ok_or("a ticket is `misa:<node>:<session>`")?;
        if node.is_empty() || session.is_empty() {
            return Err("a ticket needs both a node and a session".into());
        }
        Ok(Ticket::new(node, session))
    }
}
