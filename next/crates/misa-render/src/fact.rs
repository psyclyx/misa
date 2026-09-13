//! Formatting a typed fact for a person.
//!
//! This is the client's half of the one place a session sends a number instead of
//! words. The session says *what the number is* by naming the node's role; this
//! decides *how to write it* — a currency symbol, a rounding, a thousands suffix, a
//! locale.
//!
//! The previous system called this `value-renderers` and made it a catalog of
//! functions. Here it is a function of the role, because the roles are already the
//! vocabulary everything else uses, and because a plugin that invents
//! `value.byte-size` should get the fallback rather than an error.
//!
//! What lives here and not in a frontend: two linear frontends and one browser
//! should not each decide that a token count of 12 400 is "12.4k". What lives in a
//! frontend and not here: how the text is then drawn.

use misa_value::Value;

/// Render a fact, given the role it was sent under.
pub fn format(role: &str, value: &Value) -> String {
    match role {
        // Money is carried in micros, because a float of dollars loses cents.
        "value.money" => money(value),
        "value.tokens" => count(value.as_i64(), "tok"),
        "value.count" => count(value.as_i64(), ""),
        "value.percent" => percent(value),
        "value.ratio" => ratio(value),
        "value.duration" => duration(value),
        "value.bytes" => bytes(value.as_i64()),
        _ => plain(value),
    }
}

/// A fact with no formatter of its own: whatever the value says.
fn plain(value: &Value) -> String {
    match value {
        Value::Str(text) => text.to_string(),
        Value::Int(number) => number.to_string(),
        Value::Float(number) => format!("{number}"),
        Value::Bool(true) => "yes".into(),
        Value::Bool(false) => "no".into(),
        Value::Null => String::new(),
        other => format!("{other}"),
    }
}

/// Micros of a currency unit, as `$1.24`. Negative values are debts, which a
/// session may legitimately report for a credit-balance provider.
pub fn money(value: &Value) -> String {
    let micros = value.as_i64().unwrap_or(0);
    let sign = if micros < 0 { "-" } else { "" };
    let micros = micros.abs();
    let whole = micros / 1_000_000;
    let cents = (micros % 1_000_000) / 10_000;
    if whole == 0 && cents == 0 && micros > 0 {
        // A fraction of a cent is not zero, and "0.00" would say it was.
        return format!("{sign}<$0.01");
    }
    format!("{sign}${whole}.{cents:02}")
}

/// A count with a suffix once it stops being readable: `12.4k`, `1.2M`.
pub fn count(value: Option<i64>, unit: &str) -> String {
    let Some(number) = value else {
        return String::new();
    };
    let negative = number < 0;
    let magnitude = number.unsigned_abs();
    let written = match magnitude {
        0..=9_999 => magnitude.to_string(),
        10_000..=999_999 => format!("{:.1}k", magnitude as f64 / 1_000.0),
        1_000_000..=999_999_999 => format!("{:.1}M", magnitude as f64 / 1_000_000.0),
        _ => format!("{:.1}B", magnitude as f64 / 1_000_000_000.0),
    };
    let sign = if negative { "-" } else { "" };
    if unit.is_empty() {
        format!("{sign}{written}")
    } else {
        format!("{sign}{written} {unit}")
    }
}

pub fn percent(value: &Value) -> String {
    match value.as_f64() {
        Some(number) if number == number.trunc() => format!("{number:.0}%"),
        Some(number) => format!("{number:.1}%"),
        None => plain(value),
    }
}

/// A fraction of a whole, as a percentage. `max` is not known here, so the fact
/// carries the fraction itself.
pub fn ratio(value: &Value) -> String {
    match value.as_f64() {
        Some(fraction) => percent(&Value::Float(fraction * 100.0)),
        None => plain(value),
    }
}

/// Milliseconds, as a person says them.
pub fn duration(value: &Value) -> String {
    let Some(millis) = value.as_i64() else {
        return plain(value);
    };
    let millis = millis.max(0);
    if millis < 1_000 {
        format!("{millis}ms")
    } else if millis < 60_000 {
        format!("{:.1}s", millis as f64 / 1_000.0)
    } else if millis < 3_600_000 {
        let minutes = millis / 60_000;
        let seconds = (millis % 60_000) / 1_000;
        format!("{minutes}m{seconds:02}s")
    } else {
        let hours = millis / 3_600_000;
        let minutes = (millis % 3_600_000) / 60_000;
        format!("{hours}h{minutes:02}m")
    }
}

pub fn bytes(value: Option<i64>) -> String {
    let Some(bytes) = value else {
        return String::new();
    };
    let magnitude = bytes.unsigned_abs();
    let sign = if bytes < 0 { "-" } else { "" };
    let (scaled, unit) = match magnitude {
        0..=999 => (magnitude as f64, "B"),
        1_000..=999_999 => (magnitude as f64 / 1_000.0, "kB"),
        1_000_000..=999_999_999 => (magnitude as f64 / 1_000_000.0, "MB"),
        _ => (magnitude as f64 / 1_000_000_000.0, "GB"),
    };
    if unit == "B" {
        format!("{sign}{scaled:.0} {unit}")
    } else {
        format!("{sign}{scaled:.1} {unit}")
    }
}

/// Whether a role has a formatter of its own, for a client that wants to tell a
/// fact from a word.
pub fn is_typed(role: &str) -> bool {
    matches!(
        role,
        "value.money"
            | "value.tokens"
            | "value.count"
            | "value.percent"
            | "value.ratio"
            | "value.duration"
            | "value.bytes"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_writes_cents_and_does_not_call_a_fraction_zero() {
        assert_eq!(money(&Value::Int(1_240_000)), "$1.24");
        assert_eq!(money(&Value::Int(0)), "$0.00");
        assert_eq!(money(&Value::Int(5_000)), "<$0.01");
        assert_eq!(money(&Value::Int(-2_500_000)), "-$2.50");
        assert_eq!(money(&Value::Int(1_000_000_000)), "$1000.00");
    }

    #[test]
    fn a_count_gets_a_suffix_once_it_stops_being_readable() {
        assert_eq!(count(Some(42), ""), "42");
        assert_eq!(count(Some(9_999), "tok"), "9999 tok");
        assert_eq!(count(Some(12_400), "tok"), "12.4k tok");
        assert_eq!(count(Some(1_200_000), ""), "1.2M");
        assert_eq!(count(None, "tok"), "");
    }

    #[test]
    fn a_percentage_and_a_ratio_both_end_up_as_percentages() {
        assert_eq!(percent(&Value::Int(40)), "40%");
        assert_eq!(percent(&Value::Float(40.5)), "40.5%");
        assert_eq!(ratio(&Value::Float(0.25)), "25%");
    }

    #[test]
    fn a_duration_is_written_the_way_a_person_says_it() {
        assert_eq!(duration(&Value::Int(240)), "240ms");
        assert_eq!(duration(&Value::Int(2_400)), "2.4s");
        assert_eq!(duration(&Value::Int(65_000)), "1m05s");
        assert_eq!(duration(&Value::Int(3_600_000)), "1h00m");
    }

    #[test]
    fn bytes_scale() {
        assert_eq!(bytes(Some(512)), "512 B");
        assert_eq!(bytes(Some(2_048)), "2.0 kB");
        assert_eq!(bytes(Some(5_000_000)), "5.0 MB");
    }

    #[test]
    fn an_unformatted_role_is_whatever_it_says() {
        assert_eq!(format("session.model", &Value::str("claude")), "claude");
        assert_eq!(format("plugin.custom", &Value::Int(7)), "7");
        assert!(!is_typed("session.model"));
        assert!(is_typed("value.money"));
    }

    #[test]
    fn a_formatter_is_a_function_of_the_role_and_the_value_only() {
        // Two clients with different themes must agree on the text of a fact; they
        // differ on how they draw it. This is that claim, tested.
        let value = Value::Int(1_500_000);
        assert_eq!(format("value.money", &value), money(&value));
    }
}
