//! Endpoint identity and address helpers shared by scoped, pairing and blob protocols.
use iroh::{Endpoint, EndpointAddr, RelayMode, SecretKey, endpoint::presets};
use misa_proto::{ALPN_BLOB, Ticket};
/// Bind an endpoint.
///
/// `relay` off means the endpoint uses only addresses it can reach directly, which
/// is what a test wants and what a machine on one network wants. Real use leaves it
/// on: the relay is what makes two endpoints behind different routers meet.
pub async fn bind(secret: Option<SecretKey>, relay: bool) -> Result<Endpoint, String> {
    let mut builder = Endpoint::builder(presets::N0);
    if let Some(secret) = secret {
        builder = builder.secret_key(secret);
    }
    if !relay {
        // Direct addresses only. What a test wants, and what a machine on one
        // network wants; real use leaves the relay on, because that is what makes
        // two endpoints behind different routers meet.
        builder = builder.relay_mode(RelayMode::Disabled);
    }
    builder
        // Independent protocols share endpoint identity and admission: a ticket
        // names one node, and a client that can reach a session can fetch what its views
        // point at without being told a second address.
        .alpns(vec![
            crate::pairing::ALPN.to_vec(),
            misa_proto::scoped::ALPN.to_vec(),
            ALPN_BLOB.to_vec(),
        ])
        .bind()
        .await
        .map_err(|err| err.to_string())
}

/// The endpoint's identity, and — when there is no relay — where it listens.
///
/// With a relay, the public key alone is enough: discovery finds the peer, so a ticket stays
/// short and does not go stale when a machine's addresses change. Without one there is
/// nothing to ask, so the ticket has to carry the addresses, and it does, as
/// `id@ip:port,ip:port`.
pub fn node_of(endpoint: &Endpoint) -> String {
    let id = endpoint.id().to_string();
    let mut sockets: Vec<String> = endpoint
        .bound_sockets()
        .iter()
        // A wildcard bind is not an address anybody can dial. It is *reachable* at loopback
        // on the same machine, which is what somebody running a daemon and a client side by
        // side means by it, so that is what the ticket says. Advertising the machine's other
        // names is what the relay is for, and with a relay this branch does not run.
        .map(|socket| match socket {
            std::net::SocketAddr::V4(v4) if v4.ip().is_unspecified() => {
                format!("127.0.0.1:{}", v4.port())
            }
            std::net::SocketAddr::V6(v6) if v6.ip().is_unspecified() => String::new(),
            other => other.to_string(),
        })
        .filter(|socket| !socket.is_empty())
        .collect();
    sockets.sort();
    sockets.dedup();
    if sockets.is_empty() {
        id
    } else {
        format!("{id}@{}", sockets.join(","))
    }
}

/// Whether a ticket's node names an endpoint only this machine can reach.
///
/// A loopback address needs no help to dial, so a client that has one should not wait for a
/// relay: on a machine with no route out — which is exactly the machine that runs a daemon
/// `--no-relay` beside a client — that wait is a timeout, and a daemon a client can see would
/// be unreachable for the sake of a service nobody needed. A node with no addresses is the
/// other case: a public key alone is only dialable through discovery, which is the relay.
pub fn names_only_this_machine(node: &str) -> bool {
    let Some((_, addresses)) = node.split_once('@') else {
        return false;
    };
    let sockets: Vec<&str> = addresses
        .split(',')
        .filter(|socket| !socket.is_empty())
        .collect();
    !sockets.is_empty()
        && sockets.iter().all(|socket| {
            socket
                .parse::<std::net::SocketAddr>()
                .map(|socket| socket.ip().is_loopback())
                .unwrap_or(false)
        })
}

/// Bind an endpoint for reaching a ticket's node, with a relay only when one is needed.
///
/// The choice a client should not have to make: an endpoint that is reachable directly is
/// reached directly, and one that is not is reached through the relay. `--no-relay` on a
/// daemon is therefore not a flag every client has to be told about.
pub async fn bind_for(node: &str) -> Result<Endpoint, String> {
    bind(None, !names_only_this_machine(node)).await
}

/// Parse a ticket's node part back into something dialable.
pub fn address_of(node: &str) -> Result<EndpointAddr, String> {
    let (id, addresses) = match node.split_once('@') {
        Some((id, addresses)) => (id, Some(addresses)),
        None => (node, None),
    };
    let id: iroh::EndpointId = id
        .parse()
        .map_err(|err| format!("bad endpoint id: {err}"))?;
    let mut address = EndpointAddr::new(id);
    if let Some(addresses) = addresses {
        for socket in addresses.split(',').filter(|socket| !socket.is_empty()) {
            let socket = socket
                .parse()
                .map_err(|err| format!("`{socket}` is not an address: {err}"))?;
            address = address.with_ip_addr(socket);
        }
    }
    Ok(address)
}

/// A ticket for one session on one endpoint.
pub fn ticket(endpoint: &Endpoint, session: &str) -> Ticket {
    Ticket::new(node_of(endpoint), session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_that_names_only_this_machine_is_reached_without_a_relay() {
        // The decision a client should not have to make, and the one that decides whether a
        // daemon on a machine with no route out can be reached at all.
        assert!(names_only_this_machine("abc@127.0.0.1:5000"));
        assert!(names_only_this_machine("abc@127.0.0.1:5000,127.0.0.1:5001"));
        assert!(names_only_this_machine("abc@[::1]:5000"));
        assert!(!names_only_this_machine("abc@10.0.0.4:5000"));
        assert!(!names_only_this_machine("abc@127.0.0.1:5000,10.0.0.4:5000"));
        // A key with no address is the case discovery exists for, and discovery is the relay.
        assert!(!names_only_this_machine("abc"));
        // A node part that says nothing dialable is not a local endpoint, whatever it says.
        assert!(!names_only_this_machine("abc@not-an-address"));
        assert!(!names_only_this_machine("abc@"));
    }

    #[test]
    fn a_ticket_carries_an_identity_and_may_carry_where_to_find_it() {
        let key = SecretKey::generate();
        let node = key.public().to_string();
        // With a relay, the identity is enough: discovery does the rest.
        let address = address_of(&node).expect("a bare endpoint id parses");
        assert_eq!(address.id.to_string(), node);
        assert!(address.addrs.is_empty());

        // Without one, the addresses travel in the ticket, because there is nothing to ask.
        let with_addresses = format!("{node}@127.0.0.1:1234,10.0.0.5:5678");
        let address = address_of(&with_addresses).expect("an id with addresses parses");
        assert_eq!(address.id.to_string(), node);
        assert_eq!(address.addrs.len(), 2);

        assert!(address_of("not a key").is_err());
        assert!(address_of(&format!("{node}@not-an-address")).is_err());
    }

    #[test]
    fn a_ticket_names_both_an_endpoint_and_a_session() {
        let ticket = Ticket::new("abc", "demo");
        assert_eq!(ticket.to_string(), "misa:abc:demo");
        assert_eq!(ticket.to_string().parse::<Ticket>().unwrap(), ticket);
    }
}
