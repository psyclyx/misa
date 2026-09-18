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
use std::{collections::BTreeMap, sync::Arc};

pub type Formatter = Arc<dyn Fn(&Value) -> String + Send + Sync>;
#[derive(Clone, Default)]
pub struct Registry {
    formats: BTreeMap<String, Formatter>,
}
impl Registry {
    pub fn register(&mut self, role: &str, formatter: Formatter) -> Result<(), String> {
        if role.is_empty() || self.formats.contains_key(role) {
            return Err(format!("Duplicate or empty value renderer `{role}`"));
        }
        self.formats.insert(role.into(), formatter);
        Ok(())
    }
    pub fn format(&self, role: &str, value: &Value) -> String {
        self.formats
            .get(role)
            .map(|format| format(value))
            .unwrap_or_else(|| format(role, value))
    }
}

/// Render a fact, given the role it was sent under.
pub fn format(role: &str, value: &Value) -> String {
    match role {
        // Money is carried in micros, because a float of dollars loses cents.
        "value.money" => money(value),
        // The reference keeps the unit in the surrounding label/icon ("tokens"
        // in reports, "tok" in the status bar). Appending it here would turn
        // the shipped status indicator into `tok 12 tok`.
        "value.tokens" => count(value.as_i64(), ""),
        "value.count" => count(value.as_i64(), ""),
        "value.percent" => percent(value),
        "value.percent.remaining" => percent_remaining(value),
        "value.ratio" => ratio(value),
        "value.duration" => duration(value),
        "value.timestamp" => timestamp(value),
        "value.rate" => rate(value),
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

/// Micros of a currency unit, in the reference's money vocabulary.
///
/// The wire keeps integer micros so arithmetic cannot lose cents. The display still
/// follows the old money renderer: zero is `$0`, sub-dollar values keep four places,
/// and a value smaller than a ten-thousandth is explicitly marked as such.
pub fn money(value: &Value) -> String {
    let micros = value.as_i64().unwrap_or(0);
    let sign = if micros < 0 { "-" } else { "" };
    let micros = micros.abs();
    if micros == 0 {
        return format!("{sign}$0");
    }
    if micros < 100 {
        return format!("{sign}<$0.0001");
    }
    let dollars = micros as f64 / 1_000_000.0;
    if dollars < 1.0 {
        format!("{sign}${dollars:.4}")
    } else {
        format!("{sign}${dollars:.2}")
    }
}

/// A count with a suffix once it stops being readable: `12.4k`, `1.2M`.
pub fn count(value: Option<i64>, unit: &str) -> String {
    let Some(number) = value else {
        return String::new();
    };
    let negative = number < 0;
    let magnitude = number.unsigned_abs();
    let written = match magnitude {
        0..=999 => magnitude.to_string(),
        1_000..=999_999 => compact_scaled(magnitude as f64 / 1_000.0, "k"),
        1_000_000..=999_999_999 => compact_scaled(magnitude as f64 / 1_000_000.0, "M"),
        _ => compact_scaled(magnitude as f64 / 1_000_000_000.0, "G"),
    };
    let sign = if negative { "-" } else { "" };
    if unit.is_empty() {
        format!("{sign}{written}")
    } else {
        format!("{sign}{written} {unit}")
    }
}

fn compact_scaled(value: f64, suffix: &str) -> String {
    if value.abs() >= 10.0 {
        format!("{value:.0}{suffix}")
    } else {
        format!(
            "{}{suffix}",
            format!("{value:.1}")
                .trim_end_matches('0')
                .trim_end_matches('.')
        )
    }
}

pub fn percent(value: &Value) -> String {
    match value.as_f64() {
        Some(number) if number.is_finite() && (0.0..=100.0).contains(&number) => format!(
            "{}%",
            number.floor() as i64 + i64::from(number.fract() >= 0.5)
        ),
        None => plain(value),
        Some(number) => format!("{number}%"),
    }
}

fn percent_remaining(value: &Value) -> String {
    match value.as_f64() {
        Some(number) if number.is_finite() && (0.0..=100.0).contains(&number) => format!(
            "{}% left",
            number.floor() as i64 + i64::from(number.fract() >= 0.5)
        ),
        None => plain(value),
        Some(number) => format!("{number}% left"),
    }
}

/// A fraction of a whole, as a percentage. `max` is not known here, so the fact
/// carries the fraction itself.
pub fn ratio(value: &Value) -> String {
    if let Some(map) = value.as_map() {
        let used = map
            .get("used")
            .map_or_else(|| "?".into(), |value| count(value.as_i64(), ""));
        let limit = map
            .get("limit")
            .map_or_else(|| "?".into(), |value| count(value.as_i64(), ""));
        return format!("{used}/{limit}");
    }
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
        format!("{minutes}m {seconds}s")
    } else {
        let minutes = millis / 60_000;
        let seconds = (millis % 60_000) / 1_000;
        format!("{minutes}m {seconds}s")
    }
}

/// Unix milliseconds as the compact wall-clock fact used by transcript boundaries.
/// The reference intentionally displays the instant's compact UTC clock portion.
pub fn timestamp(value: &Value) -> String {
    let Some(millis) = value.as_i64() else {
        return plain(value);
    };
    let seconds = millis.div_euclid(1_000);
    let within = seconds.rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        within / 3_600,
        within / 60 % 60,
        within % 60
    )
}

pub fn rate(value: &Value) -> String {
    match value.as_f64() {
        Some(rate) => format!("{rate:.1} tok/s"),
        None => plain(value),
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
            | "value.percent.remaining"
            | "value.ratio"
            | "value.duration"
            | "value.timestamp"
            | "value.rate"
            | "value.bytes"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_writes_cents_and_does_not_call_a_fraction_zero() {
        assert_eq!(money(&Value::Int(1_240_000)), "$1.24");
        assert_eq!(money(&Value::Int(0)), "$0");
        assert_eq!(money(&Value::Int(5_000)), "$0.0050");
        assert_eq!(money(&Value::Int(-2_500_000)), "-$2.50");
        assert_eq!(money(&Value::Int(1_000_000_000)), "$1000.00");
    }

    #[test]
    fn a_count_gets_a_suffix_once_it_stops_being_readable() {
        assert_eq!(count(Some(42), ""), "42");
        assert_eq!(count(Some(999), "tok"), "999 tok");
        assert_eq!(count(Some(9_999), "tok"), "10k tok");
        assert_eq!(count(Some(12_400), "tok"), "12k tok");
        assert_eq!(count(Some(10_000), "tok"), "10k tok");
        assert_eq!(count(Some(1_000_000), ""), "1M");
        assert_eq!(count(Some(1_200_000), ""), "1.2M");
        assert_eq!(count(None, "tok"), "");
    }

    #[test]
    fn percentages_ratios_and_remaining_quota_use_reference_spelling() {
        assert_eq!(percent(&Value::Int(40)), "40%");
        assert_eq!(percent(&Value::Float(40.5)), "41%");
        assert_eq!(
            format("value.percent.remaining", &Value::Int(40)),
            "40% left"
        );
        assert_eq!(
            ratio(&Value::map([
                ("used", Value::Int(12_400)),
                ("limit", Value::Int(1_000_000))
            ])),
            "12k/1M"
        );
    }

    #[test]
    fn a_duration_is_written_the_way_a_person_says_it() {
        assert_eq!(duration(&Value::Int(240)), "240ms");
        assert_eq!(duration(&Value::Int(2_400)), "2.4s");
        assert_eq!(duration(&Value::Int(65_000)), "1m 5s");
        assert_eq!(duration(&Value::Int(3_600_000)), "60m 0s");
    }

    #[test]
    fn bytes_scale() {
        assert_eq!(bytes(Some(512)), "512 B");
        assert_eq!(bytes(Some(2_048)), "2.0 kB");
        assert_eq!(bytes(Some(5_000_000)), "5.0 MB");
    }

    #[test]
    fn timestamp_is_only_the_compact_clock() {
        assert_eq!(timestamp(&Value::Int(1_758_067_200_000)), "00:00:00");
        assert_eq!(timestamp(&Value::Int(-1)), "23:59:59");
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
