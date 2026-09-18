//! Domain value codec regression tests.
#[cfg(test)]
mod tests {
    use crate::preparation::candidates;
    use crate::view::{ActionOn, Span};
    use crate::{Fault, Pairing, Query, Ticket};
    use misa_value::Value;
    use serde::{Deserialize, Serialize};

    fn round_trip_cbor<T>(value: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(value, &mut bytes).unwrap();
        ciborium::de::from_reader(&bytes[..]).unwrap()
    }

    #[test]
    fn a_query_key_distinguishes_its_arguments() {
        let one = Query::new("session.view").arg(Value::Int(1));
        let two = Query::new("session.view").arg(Value::Int(2));
        let three = Query::new("session.view").arg(Value::str("1"));
        assert_ne!(one.key(), two.key());
        assert_ne!(one.key(), three.key());
        assert_eq!(one.key(), one.clone().key());
        assert_ne!(one.key(), Query::new("session.view").key());
    }

    #[test]
    fn a_ticket_is_text_a_person_can_carry() {
        let ticket = Ticket::new("abc123", "demo");
        assert_eq!(ticket.to_string(), "misa:abc123:demo");
        assert_eq!("misa:abc123:demo".parse::<Ticket>().unwrap(), ticket);
        assert!("abc123:demo".parse::<Ticket>().is_err());
        assert!("misa::demo".parse::<Ticket>().is_err());
    }

    #[test]
    fn a_pairing_string_carries_a_ticket_and_a_code_with_no_way_to_confuse_them() {
        // An address is the awkward part: it has colons, an `@`, and possibly commas, so the
        // separator has to be a character none of those can be.
        let ticket = Ticket::new("abc123@127.0.0.1:5000,10.0.0.4:5000", "demo");
        let pairing = Pairing::new(ticket.clone(), "K7QX-3M2P");
        let text = pairing.to_string();
        assert!(text.starts_with("misa-pair:"), "{text}");
        let parsed: Pairing = text.parse().expect("a pairing string");
        assert_eq!(parsed.ticket, ticket);
        assert_eq!(parsed.code, "K7QX-3M2P");
        // And a ticket on its own is not a pairing string, because it has no code.
        assert!("misa:abc123:demo".parse::<Pairing>().is_err());
        assert!("misa-pair:misa:abc123:demo".parse::<Pairing>().is_err());
        assert!("misa-pair:misa:abc123:demo#".parse::<Pairing>().is_err());
        assert!("misa:abc123:demo#code".parse::<Pairing>().is_err());
    }

    #[test]
    fn an_action_on_round_trips() {
        assert_eq!(round_trip_cbor(&ActionOn::Submit), ActionOn::Submit);
        let spans = vec![Span::link("docs", "https://example.invalid")];
        assert_eq!(round_trip_cbor(&spans), spans);
    }
    #[test]
    fn candidates_decode_the_same_way_from_either_path() {
        // A resident source's items and an on-demand reply carry one shape, so a
        // picker built for one works unchanged for the other.
        let value = Value::list([
            Value::map([
                ("value", Value::str("claude")),
                ("label", Value::str("Claude")),
                ("detail", Value::str("200k context")),
                (
                    "metadata",
                    Value::map([
                        ("context_window", Value::Int(200000)),
                        ("efforts", Value::list([Value::str("low")])),
                    ]),
                ),
            ]),
            // An item with no value names nothing and is skipped rather than
            // becoming a candidate that cannot be chosen.
            Value::map([("label", Value::str("nameless"))]),
            Value::map([("value", Value::str("gpt")), ("label", Value::str("GPT"))]),
        ]);
        let decoded = candidates(&value);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].value, "claude");
        assert_eq!(decoded[0].detail.as_deref(), Some("200k context"));
        assert_eq!(decoded[0].metadata.as_ref().unwrap().context_window, Some(200000));
        assert_eq!(decoded[0].metadata.as_ref().unwrap().efforts, ["low"]);
        assert_eq!(decoded[1].label, "GPT");
        assert!(candidates(&Value::Null).is_empty());
    }

    #[test]
    fn a_missing_argument_fault_says_which_picker_to_open() {
        let fault = Fault::argument("model", "model", Some("models"));
        assert_eq!(fault.code, "argument.required");
        assert_eq!(
            fault.data.get("command").and_then(Value::as_str),
            Some("model")
        );
        assert_eq!(
            fault.data.get("source").and_then(Value::as_str),
            Some("models")
        );
    }
}
