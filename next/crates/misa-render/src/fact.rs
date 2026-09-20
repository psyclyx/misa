//! Formatting a typed fact for a person.
//!
//! This is the client's half of the one place a session sends a number instead of
//! words. The session says *what the number is* by naming the node's role; this
//! decides *how to write it* — a currency symbol, a rounding, a thousands suffix, a
//! locale.
//!
//! The previous system called this `value-renderers` and made it a catalog of
//! functions. It is a registry here too: a role selects a formatter, an unknown
//! role falls back to the value's own text rather than an error, and a client or a
//! plugin may register another role without touching the renderer. The shipped
//! formatters are the old `values.builtins` table.
//!
//! What lives here and not in a frontend: two linear frontends and one browser
//! should not each decide that a token count of 12 400 is "12.4k". What lives in a
//! frontend and not here: how the text is then drawn. Datetime formatting takes the
//! instant as given; a relative reference is carried by the fact itself, because
//! this layer has no clock.

use misa_value::Value;
use std::{collections::BTreeMap, sync::Arc};

pub type Formatter = Arc<dyn Fn(&Value) -> String + Send + Sync>;

#[derive(Clone)]
pub struct Registry {
    formats: BTreeMap<String, Formatter>,
}

impl Registry {
    pub fn empty() -> Registry {
        Registry {
            formats: BTreeMap::new(),
        }
    }

    /// The shipped formatters, keyed by the role they answer.
    pub fn builtins() -> Registry {
        let mut registry = Registry::empty();
        let mut register = |role: &str, formatter: fn(&Value) -> String| {
            registry
                .register(role, Arc::new(formatter))
                .expect("unique shipped formatter");
        };
        register("value.text", text);
        register("value.boolean", boolean);
        register("value.number", number);
        register("value.datetime", datetime);
        register("value.sequence", sequence);
        register("value.unavailable", unavailable);
        register("value.money", money);
        register("value.tokens", tokens);
        register("value.count", count_value);
        register("value.percent", percent);
        register("value.percent.remaining", percent_remaining);
        register("value.ratio", ratio);
        register("value.duration", duration);
        register("value.timestamp", timestamp);
        register("value.rate", rate);
        register("value.bytes", bytes_value);
        registry
    }

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
            .unwrap_or_else(|| role_format(role, value))
    }
}

impl Default for Registry {
    fn default() -> Self {
        Registry::builtins()
    }
}

/// The registry the renderers use when nobody supplied one.
pub fn stock() -> &'static Registry {
    static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(Registry::builtins)
}

/// Render a fact, given the role it was sent under, using the shipped formatters.
pub fn format(role: &str, value: &Value) -> String {
    stock().format(role, value)
}

/// The role convention when no formatter is registered for a role.
fn role_format(role: &str, value: &Value) -> String {
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

/// A text fact: the string itself. A caller that used `value.text` as a generic
/// fallback still gets the value's own spelling, so it does not silently vanish.
pub fn text(value: &Value) -> String {
    match value {
        Value::Str(text) => text.to_string(),
        other => plain(other),
    }
}

/// A boolean fact, spelled as the previous system spelled it.
pub fn boolean(value: &Value) -> String {
    match value.as_bool() {
        Some(true) => "true".into(),
        Some(false) => "false".into(),
        None => plain(value),
    }
}

/// A number fact. The compact suffix is only for counts, so a plain number keeps
/// its full value.
pub fn number(value: &Value) -> String {
    match value {
        Value::Int(integer) => integer.to_string(),
        Value::Float(float) if float.is_finite() => {
            let text = format!("{float}");
            text.strip_suffix(".0").map(str::to_string).unwrap_or(text)
        }
        _ => plain(value),
    }
}

/// An unavailable fact is a word, because the reason is the point.
pub fn unavailable(value: &Value) -> String {
    value
        .as_str()
        .filter(|reason| !reason.is_empty())
        .unwrap_or("unavailable")
        .to_string()
}

/// A sequence of typed facts, rendered in order with no separator.
pub fn sequence(value: &Value) -> String {
    let empty = Vec::new();
    let items = value
        .as_list()
        .or_else(|| value.get("values").and_then(Value::as_list))
        .unwrap_or(&empty);
    let mut out = String::new();
    for item in items {
        let rendered = match item
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| !kind.is_empty())
        {
            Some(kind) => {
                let inner = item.get("value").unwrap_or(item);
                stock().format(&format!("value.{kind}"), inner)
            }
            None => plain(item),
        };
        out.push_str(&rendered);
    }
    out
}

/// A datetime fact.
///
/// The instant is seconds since the Unix epoch, or an RFC 3339 string. A fact may
/// be a record with a `prefix`, a `relative_to` reference instant, a `utc_offset`
/// in seconds for a local wall clock and a `fallback` for an instant that cannot be
/// parsed; a scalar is the instant alone. The previous system obtained local time
/// from a host capability; here the offset is part of the fact, so the formatter
/// stays pure and every surface agrees.
pub fn datetime(value: &Value) -> String {
    let (instant, prefix, relative_to, offset, fallback) = match value.as_map() {
        Some(map) => (
            map.get("value").or_else(|| map.get("instant")),
            map.get("prefix").and_then(Value::as_str).unwrap_or(""),
            map.get("relative_to").and_then(timestamp_seconds),
            map.get("utc_offset").and_then(Value::as_i64).unwrap_or(0),
            map.get("fallback").and_then(Value::as_str),
        ),
        None => (Some(value), "", None, 0, None),
    };
    let Some(seconds) = instant.and_then(timestamp_seconds) else {
        return fallback.unwrap_or("Time unavailable").to_string();
    };
    let formatted = format_utc(seconds + offset as f64);
    match relative_to {
        Some(now) => format!("{prefix}{formatted} ({})", relative_time(seconds, now)),
        None => format!("{prefix}{formatted}"),
    }
}

/// Parse a Unix-seconds number or an RFC 3339 string into seconds.
pub fn timestamp_seconds(value: &Value) -> Option<f64> {
    match value {
        Value::Int(integer) => Some(*integer as f64),
        Value::Float(float) if float.is_finite() => Some(*float),
        Value::Str(text) => parse_rfc3339(text).map(|seconds| seconds as f64),
        _ => None,
    }
}

/// The previous system's relative spelling: "now", "in 3m", "2h 5m ago".
pub fn relative_time(instant: f64, now: f64) -> String {
    let seconds = instant - now;
    let minutes = (seconds.abs() / 60.0).ceil() as i64;
    let duration = if minutes < 1 {
        "<1m".to_string()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else if minutes < 1_440 {
        format!("{}h {}m", minutes / 60, minutes % 60)
    } else {
        format!("{}d {}h", minutes / 1_440, (minutes % 1_440) / 60)
    };
    if seconds == 0.0 {
        "now".into()
    } else if seconds < 0.0 {
        format!("{duration} ago")
    } else {
        format!("in {duration}")
    }
}

/// Format an instant as `YYYY-MM-DD HH:MM:SS` in UTC.
fn format_utc(seconds: f64) -> String {
    let seconds = seconds.floor() as i64;
    let within = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        within / 3_600,
        within / 60 % 60,
        within % 60
    )
}

/// Days since the Unix epoch to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// A civil date to days since the Unix epoch (the inverse of `civil_from_days`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Parse the `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)` subset of RFC 3339.
fn parse_rfc3339(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let digits = |start: usize, len: usize| -> Option<i64> {
        text.get(start..start + len)?.parse::<i64>().ok()
    };
    if bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let (year, month, day) = (digits(0, 4)?, digits(5, 2)?, digits(8, 2)?);
    let (hour, minute, second) = (digits(11, 2)?, digits(14, 2)?, digits(17, 2)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    let mut index = 19;
    // A fractional second is present but does not change the value in whole seconds.
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
    }
    let offset = match bytes.get(index) {
        Some(b'Z') | Some(b'z') | None => 0,
        Some(sign @ (b'+' | b'-')) => {
            let sign = if *sign == b'+' { 1 } else { -1 };
            let hours = digits(index + 1, 2)?;
            if bytes.get(index + 3) != Some(&b':') {
                return None;
            }
            let minutes = digits(index + 4, 2)?;
            sign * (hours * 3_600 + minutes * 60)
        }
        _ => return None,
    };
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second - offset)
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

/// A tokens fact. The surrounding label carries the unit, as the reference does.
fn tokens(value: &Value) -> String {
    count(value.as_i64(), "")
}

/// A count fact.
fn count_value(value: &Value) -> String {
    count(value.as_i64(), "")
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
/// Follow-up: the session sends UTC integers and local-time rendering belongs to the
/// client, but there is no client-side timezone facility yet, so this stays UTC for now.
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

fn bytes_value(value: &Value) -> String {
    bytes(value.as_i64())
}

/// Whether a role has a formatter of its own, for a client that wants to tell a
/// fact from a word.
pub fn is_typed(role: &str) -> bool {
    matches!(
        role,
        "value.text"
            | "value.boolean"
            | "value.number"
            | "value.datetime"
            | "value.sequence"
            | "value.unavailable"
            | "value.money"
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

    #[test]
    fn text_boolean_and_number_keep_their_own_spelling() {
        assert_eq!(format("value.text", &Value::str("hi")), "hi");
        assert_eq!(format("value.boolean", &Value::Bool(true)), "true");
        assert_eq!(format("value.boolean", &Value::Bool(false)), "false");
        assert_eq!(format("value.number", &Value::Int(42)), "42");
        assert_eq!(format("value.number", &Value::Float(2.5)), "2.5");
        assert_eq!(format("value.number", &Value::Float(3.0)), "3");
        assert_eq!(format("value.unavailable", &Value::Null), "unavailable");
    }

    #[test]
    fn a_sequence_renders_each_typed_item_in_order() {
        let value = Value::list([
            Value::map([
                ("type", Value::str("money")),
                ("value", Value::Int(1_240_000)),
            ]),
            Value::map([
                ("type", Value::str("tokens")),
                ("value", Value::Int(12_400)),
            ]),
        ]);
        assert_eq!(format("value.sequence", &value), "$1.2412k");
    }

    #[test]
    fn datetime_parses_rfc3339_and_unix_seconds_and_shows_relative() {
        assert_eq!(
            format("value.datetime", &Value::str("1970-01-01T00:00:00Z")),
            "1970-01-01 00:00:00"
        );
        assert_eq!(
            format("value.datetime", &Value::Int(0)),
            "1970-01-01 00:00:00"
        );
        assert_eq!(
            format(
                "value.datetime",
                &Value::map([
                    ("value", Value::str("2024-01-01T00:00:00+01:00")),
                    ("prefix", Value::str("at ")),
                ])
            ),
            "at 2023-12-31 23:00:00"
        );
        assert_eq!(
            format(
                "value.datetime",
                &Value::map([
                    ("value", Value::Int(1_000)),
                    ("relative_to", Value::Int(1_000)),
                ])
            ),
            "1970-01-01 00:16:40 (now)"
        );
        assert_eq!(
            format(
                "value.datetime",
                &Value::map([("value", Value::Int(0)), ("utc_offset", Value::Int(3_600)),])
            ),
            "1970-01-01 01:00:00"
        );
        assert_eq!(
            format(
                "value.datetime",
                &Value::map([
                    ("value", Value::str("nonsense")),
                    ("fallback", Value::str("Long ago")),
                ])
            ),
            "Long ago"
        );
    }

    #[test]
    fn relative_time_matches_the_reference_spelling() {
        assert_eq!(relative_time(0.0, 0.0), "now");
        assert_eq!(relative_time(180.0, 0.0), "in 3m");
        assert_eq!(relative_time(-7_500.0, 0.0), "2h 5m ago");
        assert_eq!(relative_time(-90_000.0, 0.0), "1d 1h ago");
    }
}
