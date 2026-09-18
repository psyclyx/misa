//! What a session declares about itself.
//!
//! This is the previous system's sealed catalogs, and it is deliberately the same
//! idea: a session is a *composition*, and a composition is data. What changed is
//! that the declarations now cross a wire, so they have to say enough for a client
//! it has never met to be useful with no round trip:
//!
//! - a **command** says what it does and which arguments it needs;
//! - an **argument** says where its values come from, by naming a **source**;
//! - a **source** says whether a client should hold it (`Resident`) or ask for it
//!   (`OnDemand`).
//!
//! That is all a client needs to know that `/model` should open a picker, that
//! models are worth holding, and that a conversation search is worth asking for.
//! Nothing here says how a picker looks, what keys drive it, or when it opens —
//! those differ on every platform and none of them is the session's business.
//!
//! # Why a model's price lives here
//!
//! Because cost is a *fact about a request* and a price is a *fact about a model*,
//! and neither is presentation. The session settles an attempt with a cost in
//! micros; whether that becomes `$1.24` or `1,24 €` is the client's, which is why
//! `value.money` exists in the view vocabulary.

use misa_proto::preparation::{Arg, Source, SourceKind};
use misa_proto::view::{Choice, ChoiceMetadata, ChoicePeak, ChoicePeakWindow, ChoicePricing};
use misa_value::Value;

/// Owner-local input spelling before it is bound to an installed command or read.
pub struct ShortcutTemplate {
    pub id: String,
    pub label: String,
    pub description: String,
    pub args: Vec<Arg>,
}

impl ShortcutTemplate {
    fn new(id: &str, label: &str, description: &str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: description.into(),
            args: vec![],
        }
    }
    fn arg(mut self, arg: Arg) -> Self {
        self.args.push(arg);
        self
    }
    pub fn first_required(&self) -> Option<&Arg> {
        self.args.iter().find(|arg| arg.required)
    }
}

/// The models to offer, from what the service said and what the catalog knows.
///
/// The service is the *list* and the catalog is the *facts*, which is the previous system's
/// arrangement and the only one that survives a model being renamed by the people who serve it:
/// a row nobody has heard of is still offered, with whatever can honestly be said about it and
/// no invented price. Before the first answer from a service the static catalog is the list,
/// because a session with no models to offer is a picker with nothing in it. The list is global:
/// model values are qualified ids, so choosing one can also switch providers.
pub fn choices(discovered: &Value, provider: &str, current: &str) -> Vec<Choice> {
    // `scripted` is a test-only provider name. It may still be present in an old
    // session snapshot, but it must never become a model picker again.
    if provider == "scripted" {
        return Vec::new();
    }
    let listed = discovered.as_list().unwrap_or(&[]);
    if listed.is_empty() {
        return MODELS
            .iter()
            .map(|model| choice_of(model, current, provider))
            .collect();
    }
    let choices = listed
        .iter()
        .filter(|row| {
            let id = row.get("id").and_then(Value::as_str).unwrap_or_default();
            !is_fixture_model(id)
        })
        .filter_map(|row| {
            let id = row.get("id").and_then(Value::as_str)?;
            let row_provider = row
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or(provider);
            Some(
                match model(id).filter(|known| known.provider == row_provider) {
                    Some(known) => choice_of_discovered(&known, row, current, provider),
                    None => Choice {
                        value: format!("{row_provider}/{id}"),
                        // The value remains the stable provider/model identifier. A service's
                        // display name is presentation metadata for the row, not a second
                        // provider vocabulary.
                        label: row
                            .get("label")
                            .and_then(Value::as_str)
                            .unwrap_or(id)
                            .to_string(),
                        detail: discovered_detail(row, current, provider),
                        metadata: metadata_from_row(row),
                    },
                },
            )
        })
        .collect::<Vec<_>>();
    choices
}

fn is_fixture_model(id: &str) -> bool {
    id == "scripted" || id.starts_with("scripted-")
}

fn discovered_detail(row: &Value, current: &str, active_provider: &str) -> Option<String> {
    let mut parts = Vec::new();
    let id = row.get("id").and_then(Value::as_str).unwrap_or_default();
    let row_provider = row
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or(active_provider);
    if current == format!("{row_provider}/{id}")
        || (current == id && row_provider == active_provider)
    {
        parts.push("current".to_string());
    }
    if let Some(context) = row.get("context_window").and_then(Value::as_i64) {
        parts.push(format!("{}k context", context / 1_000));
    }
    if row
        .get("efforts")
        .and_then(Value::as_list)
        .is_some_and(|levels| !levels.is_empty())
    {
        parts.push("effort".into());
    }
    if let Some(pricing) = discovered_price_detail(row) {
        parts.push(pricing);
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn metadata_from_row(row: &Value) -> Option<ChoiceMetadata> {
    let context_window = row.get("context_window").and_then(Value::as_i64);
    let efforts = row
        .get("efforts")
        .and_then(Value::as_list)
        .map(|efforts| {
            efforts
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let pricing = pricing_from_row(row);
    let peak = peak_from_value(row.get("peak"));
    (context_window.is_some() || !efforts.is_empty() || pricing.is_some() || peak.is_some())
        .then_some(ChoiceMetadata {
            context_window,
            efforts,
            pricing,
            peak,
        })
}

fn model_metadata(model: &Model) -> ChoiceMetadata {
    ChoiceMetadata {
        context_window: Some(model.context_window),
        efforts: model.efforts.iter().map(|effort| (*effort).into()).collect(),
        pricing: (model.input_micros > 0 || model.output_micros > 0).then_some(ChoicePricing {
            input_micros_per_thousand: Some(model.input_micros),
            output_micros_per_thousand: Some(model.output_micros),
            cache_read_micros_per_thousand: None,
            cache_write_micros_per_thousand: None,
            request_micros: None,
        }),
        peak: None,
    }
}

fn pricing_from_row(row: &Value) -> Option<ChoicePricing> {
    let input = row.get("input_micros").and_then(Value::as_i64);
    let output = row.get("output_micros").and_then(Value::as_i64);
    let cache_read = row.get("cache_read_micros").and_then(Value::as_i64);
    let cache_write = row.get("cache_write_micros").and_then(Value::as_i64);
    let request = row.get("request_micros").and_then(Value::as_i64);
    (input.is_some() || output.is_some() || cache_read.is_some() || cache_write.is_some() || request.is_some())
        .then_some(ChoicePricing {
            input_micros_per_thousand: input,
            output_micros_per_thousand: output,
            cache_read_micros_per_thousand: cache_read,
            cache_write_micros_per_thousand: cache_write,
            request_micros: request,
        })
}

fn peak_from_value(value: Option<&Value>) -> Option<ChoicePeak> {
    let peak = value?;
    let multiplier = peak.get("multiplier_ppm").and_then(Value::as_i64)?;
    let windows = peak
        .get("windows")
        .and_then(Value::as_list)?
        .iter()
        .filter_map(|window| {
            let start = window.get("start_hour").and_then(Value::as_i64)?;
            let end = window.get("end_hour").and_then(Value::as_i64)?;
            Some(ChoicePeakWindow { start_hour: start, end_hour: end })
        })
        .collect::<Vec<_>>();
    let days = peak
        .get("weekdays")
        .and_then(Value::as_list)
        .map(|days| days.iter().filter_map(Value::as_i64).collect::<Vec<_>>())
        .unwrap_or_default();
    (multiplier > 0 && !windows.is_empty()).then_some(ChoicePeak {
        multiplier_ppm: multiplier,
        weekdays: days,
        windows,
    })
}

fn format_rate(rate: i64) -> String {
    let value = format!("{:.6}", rate as f64 / 1_000.0);
    value.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn format_money(micros: i64) -> String {
    let value = format!("{:.6}", micros as f64 / 1_000_000.0);
    value.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn discovered_price_detail(row: &Value) -> Option<String> {
    let input = row.get("input_micros").and_then(Value::as_i64);
    let output = row.get("output_micros").and_then(Value::as_i64);
    let cache_read = row.get("cache_read_micros").and_then(Value::as_i64);
    let cache_write = row.get("cache_write_micros").and_then(Value::as_i64);
    let request = row.get("request_micros").and_then(Value::as_i64);
    if input.is_none()
        && output.is_none()
        && cache_read.is_none()
        && cache_write.is_none()
        && request.is_none()
    {
        return None;
    }
    let mut rates = Vec::new();
    if let Some(input) = input {
        rates.push(format!("${}/1M in", format_rate(input)));
    }
    if let Some(output) = output {
        rates.push(format!("${}/1M out", format_rate(output)));
    }
    if let Some(cache_read) = cache_read {
        rates.push(format!("${}/1M cache read", format_rate(cache_read)));
    }
    if let Some(cache_write) = cache_write {
        rates.push(format!("${}/1M cache write", format_rate(cache_write)));
    }
    if let Some(request) = request {
        rates.push(format!("${} request", format_money(request)));
    }
    if let Some(peak) = peak_price_detail(row.get("peak")) {
        rates.push(peak);
    }
    Some(rates.join(" · "))
}

fn peak_price_detail(value: Option<&Value>) -> Option<String> {
    let peak = value?;
    let multiplier = peak
        .get("multiplier_ppm")
        .and_then(Value::as_i64)
        .filter(|multiplier| *multiplier > 0 && *multiplier != 1_000_000)?;
    let windows = peak
        .get("windows")
        .and_then(Value::as_list)?
        .iter()
        .filter_map(|window| {
            let start = window.get("start_hour").and_then(Value::as_i64)?;
            let end = window.get("end_hour").and_then(Value::as_i64)?;
            Some(format!("{start:02}–{end:02}"))
        })
        .collect::<Vec<_>>();
    if windows.is_empty() {
        return None;
    }
    Some(format!(
        "{}× peak {} UTC",
        multiplier as f64 / 1_000_000.0,
        windows.join(", ")
    ))
}

/// One model, as a candidate a picker can show.
fn choice_of(model: &Model, current: &str, active_provider: &str) -> Choice {
    let current = current == format!("{}/{}", model.provider, model.id)
        || (current == model.id && model.provider == active_provider);
    let detail = format!(
        "{}{}k context{}",
        if current { "current · " } else { "" },
        model.context_window / 1_000,
        if model.efforts.is_empty() {
            ""
        } else {
            " · effort"
        },
    );
    let qualified = format!("{}/{}", model.provider, model.id);
    Choice {
        value: qualified.clone(),
        label: qualified,
        detail: Some(detail),
        metadata: Some(model_metadata(model)),
    }
}

fn choice_of_discovered(
    model: &Model,
    row: &Value,
    current: &str,
    active_provider: &str,
) -> Choice {
    let mut choice = choice_of(model, current, active_provider);
    choice.detail = discovered_detail(row, current, active_provider).or(choice.detail);
    choice.metadata = metadata_from_row(row).or(choice.metadata);
    choice
}

/// One model a session can be talking to.
#[derive(Clone, Copy, Debug)]
pub struct Model {
    pub id: &'static str,
    pub label: &'static str,
    pub provider: &'static str,
    /// The wire protocol the provider speaks.
    pub api: &'static str,
    pub context_window: i64,
    /// The reasoning levels this model accepts, in the provider's order.
    pub efforts: &'static [&'static str],
    /// Micros per thousand input tokens.
    pub input_micros: i64,
    /// Micros per thousand output tokens.
    pub output_micros: i64,
    /// Provider policy for a fresh selection; absent means no default effort.
    pub default_effort: Option<&'static str>,
}

/// The shipped catalog.
///
/// The resident models that exist before a provider's discovery response arrives.
///
/// Discovery replaces the rows for providers whose service owns the catalog. The
/// static rows are therefore only for providers with a stable local catalog (the
/// Claude Code and Codex handoffs), not a test provider and not a guessed list for every API.
pub const MODELS: &[Model] = &[
    Model {
        id: "claude-fable-5-1",
        label: "Claude Fable 5.1",
        provider: "claude",
        api: "claude.cli",
        context_window: 1_000_000,
        efforts: &["low", "medium", "high", "max"],
        input_micros: 0,
        output_micros: 0,
        default_effort: Some("high"),
    },
    Model {
        id: "claude-opus-5",
        label: "Claude Opus 5",
        provider: "claude",
        api: "claude.cli",
        context_window: 200_000,
        efforts: &["low", "medium", "high", "max"],
        input_micros: 0,
        output_micros: 0,
        default_effort: Some("high"),
    },
    Model {
        id: "claude-sonnet-5",
        label: "Claude Sonnet 5",
        provider: "claude",
        api: "claude.cli",
        context_window: 1_000_000,
        efforts: &["low", "medium", "high", "max"],
        input_micros: 0,
        output_micros: 0,
        default_effort: Some("high"),
    },
    Model {
        id: "claude-haiku-4-5-20251001",
        label: "Claude Haiku 4.5",
        provider: "claude",
        api: "claude.cli",
        context_window: 200_000,
        efforts: &["low", "medium", "high", "max"],
        input_micros: 0,
        output_micros: 0,
        default_effort: Some("high"),
    },
    Model {
        id: "gpt-5.4",
        label: "GPT-5.4 (ChatGPT)",
        provider: "openai-codex",
        api: "openai.responses",
        context_window: 1_000_000,
        efforts: &["low", "medium", "high", "xhigh"],
        input_micros: 0,
        output_micros: 0,
        default_effort: Some("medium"),
    },
    Model {
        id: "gpt-5.3-codex",
        label: "GPT-5.3 Codex",
        provider: "openai-codex",
        api: "openai.responses",
        context_window: 400_000,
        efforts: &["low", "medium", "high", "xhigh"],
        input_micros: 0,
        output_micros: 0,
        default_effort: Some("medium"),
    },
];

/// Reasoning efforts, weakest first. Only offered by a model that takes one.
pub const EFFORTS: &[&str] = &["low", "medium", "high", "max"];

/// The model a session starts on.
pub const DEFAULT_MODEL: &str = "claude-sonnet-5";

pub fn model(id: &str) -> Option<Model> {
    MODELS.iter().copied().find(|model| model.id == id)
}

/// The effort ladder for the selected model. Static facts and live provider facts
/// use one query-facing shape, so a client does not need to know whether a model
/// came from a release or from discovery.
pub fn effort_levels(discovered: &Value, provider: &str, id: &str) -> Vec<String> {
    if let Some(model) = model(id).filter(|model| model.provider == provider) {
        return model
            .efforts
            .iter()
            .map(|level| (*level).to_string())
            .collect();
    }
    discovered
        .as_list()
        .unwrap_or(&[])
        .iter()
        .find(|row| {
            row.get("provider").and_then(Value::as_str) == Some(provider)
                && row.get("id").and_then(Value::as_str) == Some(id)
        })
        .and_then(|row| row.get("efforts"))
        .and_then(Value::as_list)
        .map(|levels| {
            levels
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve the provider's starting effort for a model. Live model facts win over
/// static rows because discovery is authoritative for a provider's current model
/// policy; static handoffs retain their shipped defaults.
pub fn default_effort(discovered: &Value, provider: &str, id: &str) -> Option<String> {
    discovered
        .as_list()
        .unwrap_or(&[])
        .iter()
        .find(|row| {
            row.get("provider").and_then(Value::as_str) == Some(provider)
                && row.get("id").and_then(Value::as_str) == Some(id)
        })
        .and_then(|row| row.get("default_effort").and_then(Value::as_str))
        .map(str::to_string)
        .or_else(|| {
            model(id)
                .filter(|model| model.provider == provider)
                .and_then(|model| model.default_effort.map(str::to_string))
        })
}

pub fn discovered_supports_effort(discovered: &Value, provider: &str, id: &str) -> bool {
    !effort_levels(discovered, provider, id).is_empty()
}

pub fn default_model() -> Model {
    model(DEFAULT_MODEL).unwrap_or(MODELS[0])
}

/// What a settled attempt cost, from the model's own prices.
///
/// Rounded to a micro, which is the smallest unit anything here records. A model
/// that is not in the catalog costs nothing, which is the honest answer for a
/// scripted one: it is not that the cost is unknown, it is that there is none.
pub fn cost_micros(model_id: &str, input_tokens: i64, output_tokens: i64) -> i64 {
    let Some(model) = model(model_id) else {
        return 0;
    };
    let input = input_tokens.max(0) * model.input_micros / 1_000;
    let output = output_tokens.max(0) * model.output_micros / 1_000;
    input + output
}

pub fn discovered_cost_micros(
    discovered: &Value,
    provider: &str,
    model_id: &str,
    input_tokens: i64,
    output_tokens: i64,
) -> i64 {
    discovered_cost_micros_at(
        discovered,
        provider,
        model_id,
        input_tokens,
        output_tokens,
        0,
        0,
        None,
    )
}

/// Estimate a discovered model's cost using the pricing captured when the
/// provider was queried. Rates are integer micro-USD per thousand tokens; a
/// request charge is integer micro-USD per request. A peak schedule is resolved
/// against the request start, so a long request cannot change price tier halfway
/// through.
pub fn discovered_cost_micros_at(
    discovered: &Value,
    provider: &str,
    model_id: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    started_ms: Option<i64>,
) -> i64 {
    let Some(row) = discovered.as_list().unwrap_or(&[]).iter().find(|row| {
        row.get("provider").and_then(Value::as_str) == Some(provider)
            && row.get("id").and_then(Value::as_str) == Some(model_id)
    }) else {
        return cost_micros(model_id, input_tokens, output_tokens);
    };
    let multiplier = peak_multiplier(row.get("peak"), started_ms);
    let input = token_cost(
        input_tokens,
        row.get("input_micros").and_then(Value::as_i64),
        multiplier,
    );
    let output = token_cost(
        output_tokens,
        row.get("output_micros").and_then(Value::as_i64),
        multiplier,
    );
    let cache_read = token_cost(
        cache_read_tokens,
        row.get("cache_read_micros").and_then(Value::as_i64),
        multiplier,
    );
    let cache_write = token_cost(
        cache_write_tokens,
        row.get("cache_write_micros").and_then(Value::as_i64),
        multiplier,
    );
    let request = row
        .get("request_micros")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .saturating_mul(multiplier)
        / 1_000_000;
    input
        .saturating_add(output)
        .saturating_add(cache_read)
        .saturating_add(cache_write)
        .saturating_add(request)
}

fn token_cost(tokens: i64, rate: Option<i64>, multiplier: i64) -> i64 {
    let Some(rate) = rate else { return 0; };
    ((tokens.max(0) as i128 * rate.max(0) as i128 * multiplier.max(0) as i128)
        / 1_000
        / 1_000_000)
        .min(i64::MAX as i128) as i64
}

fn peak_multiplier(peak: Option<&Value>, started_ms: Option<i64>) -> i64 {
    let Some(peak) = peak else { return 1_000_000; };
    let Some(started_ms) = started_ms else { return 1_000_000; };
    let multiplier = peak
        .get("multiplier_ppm")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .unwrap_or(1_000_000);
    let seconds = started_ms.div_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let hour = seconds.rem_euclid(86_400) / 3_600;
    let weekday = (days + 4).rem_euclid(7) + 1;
    let weekday_allowed = peak
        .get("weekdays")
        .and_then(Value::as_list)
        .is_none_or(|days| {
            days.iter()
                .any(|day| day.as_i64() == Some(weekday))
        });
    let in_window = peak
        .get("windows")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .iter()
        .any(|window| {
            let start = window.get("start_hour").and_then(Value::as_i64);
            let end = window.get("end_hour").and_then(Value::as_i64);
            let (Some(start), Some(end)) = (start, end) else { return false; };
            if end < start {
                hour >= start || hour < end
            } else {
                hour >= start && hour < end
            }
        });
    if weekday_allowed && in_window {
        multiplier
    } else {
        1_000_000
    }
}

/// The sources a session declares.
pub fn sources() -> Vec<Source> {
    use misa_proto::{
        Query,
        completion::*,
        observation::{Encoding, Member},
    };
    [
        (MODELS, "Models", SourceKind::Resident, MODELS_QUERY),
        (
            EFFORT,
            "Reasoning effort",
            SourceKind::Resident,
            EFFORT_QUERY,
        ),
        (COMMANDS, "Commands", SourceKind::Resident, COMMANDS_QUERY),
        (
            PROVIDERS,
            "Providers",
            SourceKind::Resident,
            PROVIDERS_QUERY,
        ),
        (
            CONVERSATIONS,
            "Conversations",
            SourceKind::OnDemand,
            misa_proto::preparation::SEARCH,
        ),
    ]
    .into_iter()
    .map(|(id, label, kind, query_id)| {
        let mut query = Query::new(query_id);
        if kind == SourceKind::OnDemand {
            query = query
                .arg(Value::str(id))
                .arg(Value::str(""))
                .arg(Value::Int(
                    misa_proto::preparation::DEFAULT_CANDIDATES as i64,
                ));
        }
        Source {
            id: id.into(),
            label: label.into(),
            kind,
            member: Member {
                query,
                contract: format!("{query_id}@1"),
                encoding: Encoding::Value,
                optional: false,
            },
        }
    })
    .collect()
}

/// A source a client should hold, and the query it is read from.
pub fn resident_query(source: &str) -> Option<String> {
    let source = sources().into_iter().find(|entry| entry.id == source)?;
    match source.kind {
        SourceKind::Resident => Some(source.member.query.id),
        SourceKind::OnDemand => None,
    }
}

/// The commands a session declares.
///
/// Each one is an operation on the *session* or on a capability behind it. What is here is
/// also what a client offers: the declaration is the whole of what a palette, a completion,
/// and a hint read, so a command the loop handles and this list omits is a command nobody can
/// run — which is a worse answer than not having it.
///
/// Read shortcuts bind to finite owner exports. Each client owns the resulting
/// report presentation and can independently choose another exported representation.
pub fn commands() -> Vec<ShortcutTemplate> {
    use ShortcutTemplate as Command;
    vec![
        Command::new("clear", "Clear", "Reset conversation and token usage")
            .arg(Arg::new("reason", "Reason")),
        Command::new(
            "compact",
            "Compact",
            "Summarize and replace the conversation history",
        ),
        Command::new("model", "Model", "Choose the active model")
            .arg(Arg::new("model", "Model").required().from("models")),
        Command::new("effort", "Effort", "Choose model reasoning effort")
            .arg(Arg::new("level", "Level").required().from("effort")),
        Command::new("status", "Status", "Show provider login state").arg(
            Arg::new("provider", "Provider")
                .required()
                .from("providers"),
        ),
        Command::new("usage", "Usage", "Show token and subscription usage"),
        // A credential is a slot. `/login` with a key opens a panel with one secret field;
        // with a service that issues tokens it starts a device flow and shows a code, which is
        // why the two are one command and not two.
        Command::new("login", "Log in", "Log in to a provider")
            .arg(
                Arg::new("provider", "Provider")
                    .required()
                    .from("providers"),
            )
            .arg(Arg::new("account", "Account")),
        Command::new("logout", "Log out", "Log out of a provider")
            .arg(
                Arg::new("provider", "Provider")
                    .required()
                    .from("providers"),
            )
            .arg(Arg::new("account", "Account")),
        Command::new("account", "Account", "Switch the account a provider uses")
            .arg(
                Arg::new("provider", "Provider")
                    .required()
                    .from("providers"),
            )
            .arg(Arg::new("account", "Account").required()),
        // A path, not a hash: the file is read where the daemon is, which is the only place a
        // path means anything. A client that has the bytes uploads them instead.
        Command::new(
            "attach",
            "Attach",
            "Attach a file from the daemon's machine",
        )
        .arg(Arg::new("path", "Path").required()),
        Command::new("image", "Image", "Attach a PNG or JPEG file")
            .arg(Arg::new("path", "Path").required()),
        Command::new("resume", "Resume", "Resume a stored conversation")
            .arg(Arg::new("conversation", "Conversation").from("conversations")),
    ]
}

/// What a tool looks like to a model.
#[derive(Clone, Debug)]
pub struct ToolDecl {
    pub name: &'static str,
    pub description: &'static str,
    /// The JSON Schema the model is given. A tool without one cannot be called
    /// reliably, so every declared tool has one.
    pub schema: &'static str,
}

/// The tools the shipped composition declares to a model.
///
/// Each name must have an implementation in the kernel. `Shipped` is checked by a
/// test against `misa_kernel::tools::shipped`, because a declaration a kernel
/// cannot honour is a model being told about a tool that will always fail.
pub const TOOLS: &[ToolDecl] = &[
    ToolDecl {
        name: "read_file",
        description: "Read a text file, optionally a window of its numbered lines.",
        schema: r#"{"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer"},"max_lines":{"type":"integer"}},"required":["path"]}"#,
    },
    ToolDecl {
        name: "write_file",
        description: "Write a text file, creating its directory if it does not exist.",
        schema: r#"{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}"#,
    },
    ToolDecl {
        name: "list_directory",
        description: "List a directory, sorted, marking directories with a trailing slash.",
        schema: r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}"#,
    },
    ToolDecl {
        name: "shell",
        description: "Run a command with `sh -lc`. Its output goes to a log file, and the answer \
                 names that file. If the command is still running after `wait_ms` — or if \
                 `background` is true, which is what to use for anything long — the answer comes \
                 back at once with its pid and its log, and the session reports when it finishes. \
                 It is only stopped at `timeout_ms`, which is ten minutes by default. Read or tail \
                 the log to follow a command that is still going; do not start it again.",
        schema: r#"{"type":"object","properties":{"command":{"type":"string"},"background":{"type":"boolean"},"wait_ms":{"type":"integer"},"timeout_ms":{"type":"integer"}},"required":["command"]}"#,
    },
    ToolDecl {
        name: "echo",
        description: "Print the arguments back, unchanged. The smallest tool there is.",
        schema: r#"{"type":"object","properties":{}}"#,
    },
];

/// The tools, as the value a provider request carries.
pub fn tool_schemas() -> misa_value::Value {
    misa_value::Value::list(TOOLS.iter().map(|tool| {
        misa_value::Value::map([
            ("name", misa_value::Value::str(tool.name)),
            ("description", misa_value::Value::str(tool.description)),
            ("input_schema", misa_value::Value::str(tool.schema)),
        ])
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_declared_model_is_reachable_by_id() {
        for entry in MODELS {
            assert!(model(entry.id).is_some());
            assert!(!entry.label.is_empty(), "`{}` has no label", entry.id);
            assert!(entry.context_window > 0);
        }
        assert_eq!(default_model().id, DEFAULT_MODEL);
    }

    #[test]
    fn a_cost_is_a_function_of_tokens_and_the_models_own_prices() {
        // The shipped models are free, so the arithmetic is checked against the
        // shape a real one takes rather than against a scripted row.
        let real = Model {
            id: "x",
            label: "x",
            provider: "p",
            api: "a",
            context_window: 1,
            efforts: &[],
            input_micros: 3_000,
            output_micros: 15_000,
            default_effort: None,
        };
        assert_eq!(real.input_micros, 3_000);
        assert_eq!(cost_micros("scripted-1", 1_000_000, 1_000_000), 0);
        assert_eq!(cost_micros("no-such-model", 1_000, 1_000), 0);
    }

    #[test]
    fn discovered_prices_include_request_costs_and_time_based_peak_rates() {
        let discovered = Value::list([Value::map([
            ("provider", Value::str("deepseek")),
            ("id", Value::str("deepseek-flash")),
            ("input_micros", Value::Int(150)),
            ("output_micros", Value::Int(600)),
            ("request_micros", Value::Int(10_000)),
            (
                "peak",
                Value::map([
                    ("multiplier_ppm", Value::Int(2_000_000)),
                    ("weekdays", Value::list([Value::Int(5)])),
                    (
                        "windows",
                        Value::list([Value::map([
                            ("start_hour", Value::Int(1)),
                            ("end_hour", Value::Int(4)),
                        ])]),
                    ),
                ]),
            ),
        ])]);
        // 1970-01-01 is Thursday (weekday 5), 02:00 UTC: peak pricing applies.
        assert_eq!(
            discovered_cost_micros_at(
                &discovered,
                "deepseek",
                "deepseek-flash",
                1_000_000,
                1_000_000,
                0,
                0,
                Some(2 * 3_600_000),
            ),
            1_520_000
        );
        // The same request at 12:00 UTC is off peak.
        assert_eq!(
            discovered_cost_micros_at(
                &discovered,
                "deepseek",
                "deepseek-flash",
                1_000_000,
                1_000_000,
                0,
                0,
                Some(12 * 3_600_000),
            ),
            760_000
        );
    }

    #[test]
    fn a_command_that_cannot_run_without_an_argument_says_so() {
        let model = commands()
            .into_iter()
            .find(|command| command.id == "model")
            .expect("a model command");
        let required = model.first_required().expect("a required argument");
        assert_eq!(required.name, "model");
        assert_eq!(required.source.as_deref(), Some("models"));
        // And the source it names is one this session declares.
        assert!(resident_query("models").is_some());
    }

    #[test]
    fn model_discovery_is_internal_and_not_a_user_command() {
        assert!(
            commands().into_iter().all(|command| command.id != "models"),
            "the models source must not become a standalone /models command"
        );
    }

    #[test]
    fn an_optional_argument_is_not_required_and_may_have_no_source() {
        let clear = commands()
            .into_iter()
            .find(|command| command.id == "clear")
            .expect("a clear command");
        let reason = clear.args.first().expect("an argument");
        assert!(!reason.required);
        assert!(reason.source.is_none());
        assert!(clear.first_required().is_none());
    }

    #[test]
    fn a_resident_source_has_a_query_and_an_on_demand_one_does_not() {
        assert_eq!(
            resident_query("models").as_deref(),
            Some("completion.models")
        );
        assert_eq!(
            resident_query("commands").as_deref(),
            Some("completion.commands")
        );
        assert!(
            resident_query("conversations").is_none(),
            "an on-demand source was offered as something to hold"
        );
    }

    #[test]
    fn every_declared_tool_has_a_schema_a_model_could_validate_against() {
        for tool in TOOLS {
            assert!(
                tool.schema.starts_with('{'),
                "`{}` has no schema",
                tool.name
            );
            assert!(
                tool.schema.contains("type"),
                "`{}`'s schema says nothing",
                tool.name
            );
            assert!(!tool.description.is_empty());
        }
        let schemas = tool_schemas();
        assert_eq!(
            schemas.as_list().map(<[misa_value::Value]>::len),
            Some(TOOLS.len())
        );
    }

    #[test]
    fn every_declared_tool_is_one_the_shipped_kernel_can_run() {
        // A declared tool the kernel cannot honour is a model being told about
        // something that will always fail.
        // The tools, as the kernel builds them: the shell needs somewhere to put its logs and
        // somewhere to report a command that outlives its call, and neither is the session's.
        let (events, _reports) = tokio::sync::mpsc::unbounded_channel();
        let shipped =
            misa_kernel::tools::shipped(&misa_kernel::tools::default_shell_dir(), &events);
        let runnable: Vec<&str> = shipped.iter().map(|tool| tool.name()).collect();
        for tool in TOOLS {
            assert!(
                runnable.contains(&tool.name),
                "`{}` is declared to a model but the kernel has no implementation",
                tool.name
            );
        }
    }

    #[test]
    fn the_effort_levels_are_ordered_and_named() {
        assert_eq!(EFFORTS.first(), Some(&"low"));
        assert_eq!(EFFORTS.last(), Some(&"max"));
    }

    #[test]
    fn model_completion_contains_qualified_models_from_every_provider() {
        let catalogue = Value::list([
            Value::map([
                ("provider", Value::str("claude")),
                ("id", Value::str("claude-sonnet-5")),
            ]),
            Value::map([
                ("provider", Value::str("deepseek")),
                ("id", Value::str("deepseek-flash")),
            ]),
        ]);
        let choices = choices(&catalogue, "deepseek", "deepseek-flash");
        assert_eq!(
            choices
                .iter()
                .map(|choice| choice.value.as_str())
                .collect::<Vec<_>>(),
            ["claude/claude-sonnet-5", "deepseek/deepseek-flash"]
        );
    }

    #[test]
    fn discovered_model_preview_preserves_dynamic_pricing_and_peak_schedule() {
        let catalogue = Value::list([Value::map([
            ("provider", Value::str("deepseek")),
            ("id", Value::str("deepseek-flash")),
            ("label", Value::str("DeepSeek Flash")),
            ("context_window", Value::Int(64_000)),
            ("efforts", Value::list([Value::str("low"), Value::str("high")])),
            ("input_micros", Value::Int(150)),
            ("output_micros", Value::Int(600)),
            ("cache_read_micros", Value::Int(3)),
            ("request_micros", Value::Int(10_000)),
            (
                "peak",
                Value::map([
                    ("multiplier_ppm", Value::Int(2_000_000)),
                    ("weekdays", Value::list([Value::Int(5)])),
                    (
                        "windows",
                        Value::list([Value::map([
                            ("start_hour", Value::Int(1)),
                            ("end_hour", Value::Int(4)),
                        ])]),
                    ),
                ]),
            ),
        ])]);
        let choice = choices(&catalogue, "deepseek", "").pop().unwrap();
        assert_eq!(choice.value, "deepseek/deepseek-flash");
        let metadata = choice.metadata.expect("model facts");
        assert_eq!(metadata.context_window, Some(64_000));
        assert_eq!(metadata.efforts, ["low", "high"]);
        assert_eq!(metadata.pricing.unwrap().input_micros_per_thousand, Some(150));
        let peak = metadata.peak.expect("peak schedule");
        assert_eq!(peak.multiplier_ppm, 2_000_000);
        assert_eq!(peak.weekdays, [5]);
        assert_eq!(peak.windows[0].start_hour, 1);
    }

    #[test]
    fn an_old_scripted_catalogue_cannot_reappear_in_the_model_picker() {
        let old_catalogue = Value::list([Value::map([
            ("provider", Value::str("scripted")),
            ("id", Value::str("scripted-1")),
        ])]);
        assert!(choices(&old_catalogue, "scripted", "").is_empty());
        assert!(choices(&old_catalogue, "claude", "").is_empty());

        let unqualified_old_catalogue = Value::list([Value::map([
            ("provider", Value::str("claude")),
            ("id", Value::str("scripted-chatty")),
        ])]);
        assert!(choices(&unqualified_old_catalogue, "claude", "").is_empty());
    }
}
