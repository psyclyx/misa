//! Provider quota facts. Endpoints, credential slots and response dialects stay here.
use crate::{
    http::{Credential, Request},
    provider::Provider,
};
use misa_value::Value;
use serde_json::{Value as Json, json};

pub fn request(provider: &dyn Provider) -> Result<Request, String> {
    let mut request = match provider.id() {
        "claude" => Request::get("https://api.anthropic.com/api/oauth/usage"),
        "openai-codex" => Request::get("https://chatgpt.com/backend-api/wham/usage"),
        "kimi" | "kimi-coding" => Request::get(format!(
            "{}/usages",
            provider.api_base().ok_or("provider has no API base")?.trim_end_matches('/')
        )),
        _ => return Err("Provider quota is unavailable".into()),
    };
    request.credential = Some(match provider.id() {
        "claude" => Credential::bearer("claude"),
        "openai-codex" => Credential::bearer("openai-codex").with_account_header("chatgpt-account-id"),
        _ => provider.credential().ok_or("provider has no credential slot")?,
    });
    if provider.id() == "claude" {
        request.headers.push(("anthropic-beta".into(), "oauth-2025-04-20".into()));
    }
    Ok(request)
}
/// Fetch a complete quota snapshot; reset-credit detail failure leaves its known count intact.
pub async fn fetch(provider: &dyn Provider, http: &crate::Http) -> Result<Value, String> {
    let request = request(provider)?;
    let response = http.send(&request).await?;
    if !response.ok() {
        return Err("usage request failed".into());
    }
    let mut data = response.json().ok_or("invalid usage response")?;
    if provider.id() == "openai-codex"
        && number(&data["rate_limit_reset_credits"]["available_count"]).is_some_and(|n| n > 0.0)
    {
        let mut details = Request::get("https://chatgpt.com/backend-api/wham/rate-limit-reset-credits");
        details.credential = request.credential;
        if let Ok(response) = http.send(&details).await {
            if response.ok() {
                if let Some(details) = response.json() {
                    data["reset_credit_details"] = details;
                }
            }
        }
    }
    Ok(parse(provider.id(), &data))
}

fn number(value: &Json) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.parse().ok()).filter(|n: &f64| n.is_finite() && *n >= 0.0)
}
fn window(id: &str, label: &str, data: &Json, percent: Option<&str>) -> Option<Json> {
    if !data.is_object() {
        return None;
    }
    let used = percent.and_then(|key| number(&data[key])).or_else(|| number(&data["used"]));
    let limit = if percent.is_some() { used.map(|_| 100.0) } else { number(&data["limit"]) };
    let remaining = number(&data["remaining"]).or_else(|| Some((limit? - used?).max(0.0)));
    let used = used.or_else(|| Some((limit? - remaining?).max(0.0)));
    if percent == Some("utilization") && used.is_some_and(|n| n > 100.0) {
        return None;
    }
    let reset = data
        .get("resets_at")
        .or_else(|| data.get("resetAt"))
        .or_else(|| data.get("reset_at"))
        .filter(|value| value.is_string() || number(value).is_some())
        .cloned()
        .unwrap_or(Json::Null);
    if used.is_none() && remaining.is_none() && reset.is_null() {
        return None;
    }
    Some(
        json!({"id":id,"label":label,"unit":if percent.is_some() { "percent" } else { "count" },"used":used,"limit":limit,"remaining":remaining,"reset":reset,"reset_after_seconds":number(&data["reset_after_seconds"])}),
    )
}
/// Normalize only quota fields: account IDs, email and raw errors never become session state.
pub fn parse(provider: &str, data: &Json) -> Value {
    let mut windows = Vec::new();
    let mut credits = Json::Null;
    match provider {
        "claude" if data["rate_limits_available"] == false => {}
        "claude" => {
            let limits = data.get("rate_limits").unwrap_or(data);
            if let Some(object) = limits.as_object() {
                for (id, entry) in object {
                    if id == "extra_usage" {
                        let decimals = entry["decimal_places"].as_u64().filter(|n| *n <= 9).unwrap_or(2) as i32;
                        credits = json!({"enabled":entry["is_enabled"].as_bool(),"currency":entry["currency"].as_str().unwrap_or("USD"),"limit":number(&entry["monthly_limit"]).map(|n| n / 10f64.powi(decimals)),"used":number(&entry["used_credits"]).map(|n| n / 10f64.powi(decimals))});
                    } else if id == "model_scoped" {
                        for (i, model) in entry.as_array().into_iter().flatten().enumerate() {
                            if let Some(w) = window(
                                &format!("model.{i}"),
                                model["display_name"].as_str().unwrap_or("Model"),
                                model,
                                Some("utilization"),
                            ) {
                                windows.push(w);
                            }
                        }
                    } else if let Some(w) = window(id, id, entry, Some("utilization")) {
                        windows.push(w);
                    }
                }
            }
        }
        "openai-codex" => {
            let mut groups = vec![("Codex".to_string(), &data["rate_limit"])];
            groups.push(("Code review".into(), &data["code_review_rate_limit"]));
            for (i, extra) in data["additional_rate_limits"].as_array().into_iter().flatten().enumerate() {
                groups.push((
                    extra["limit_name"].as_str().map(str::to_string).unwrap_or(format!("Additional quota {}", i + 1)),
                    &extra["rate_limit"],
                ));
            }
            for (i, (label, group)) in groups.iter().enumerate() {
                for key in ["primary_window", "secondary_window"] {
                    let duration = match group[key]["limit_window_seconds"].as_u64() {
                        Some(18000) => "5h",
                        Some(604800) => "7d",
                        _ => key,
                    };
                    if let Some(w) = window(
                        &format!("{i}.{key}"),
                        &format!("{label} · {duration}"),
                        &group[key],
                        Some("used_percent"),
                    ) {
                        windows.push(w);
                    }
                }
            }
            if let Some(c) = data.get("credits").filter(|v| v.is_object()) {
                credits = json!({"remaining":number(&c["balance"]),"unlimited":c["unlimited"].as_bool(),"has_credits":c["has_credits"].as_bool()});
            }
        }
        "kimi" | "kimi-coding" => {
            if let Some(w) = window("usage", "Kimi", &data["usage"], None) {
                windows.push(w);
            }
            for (i, item) in data["limits"].as_array().into_iter().flatten().enumerate() {
                if let Some(w) =
                    window(&format!("limit.{i}"), item["name"].as_str().unwrap_or("Quota"), &item["detail"], None)
                {
                    windows.push(w);
                }
            }
        }
        _ => {}
    }
    let reset_count = number(&data["rate_limit_reset_credits"]["available_count"]);
    let reset_credits: Vec<Json> = data["reset_credit_details"]["credits"].as_array().into_iter().flatten().filter_map(|credit| {
        let status = credit["status"].as_str()?;
        let granted = &credit["granted_at"];
        if !granted.is_string() && number(granted).is_none() { return None; }
        let expires = &credit["expires_at"];
        Some(json!({"label":credit["title"].as_str().unwrap_or("Quota reset"),"status":status,"granted":granted,"expires":if expires.is_string() || number(expires).is_some() { expires.clone() } else { Json::Null }}))
    }).collect();
    serde_json::from_value(json!({"unavailable":windows.is_empty() && credits.is_null(),"windows":windows,"credits":credits,"reset_count":reset_count,"reset_credits":reset_credits,"plan":data.get("plan_type").or_else(|| data.get("subscription_type")).and_then(Json::as_str)})).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parsed(provider: &str, data: Json) -> Json {
        serde_json::to_value(parse(provider, &data)).unwrap()
    }
    #[test]
    fn claude_windows_zero_invalid_and_minor_credits() {
        let facts = parsed(
            "claude",
            json!({"rate_limits":{"five_hour":{"utilization":0,"resets_at":"2026-09-08T00:00:00Z"},"seven_day":{"utilization":75},"model_scoped":[{"display_name":"Fable","utilization":10}],"extra_usage":{"is_enabled":false,"currency":"USD","decimal_places":2,"monthly_limit":12345,"used_credits":0}}}),
        );
        assert_eq!(facts["windows"].as_array().unwrap().len(), 3);
        assert_eq!(facts["windows"][0]["remaining"], 100.0);
        assert_eq!(facts["credits"]["limit"], 123.45);
        assert_eq!(facts["credits"]["enabled"], false);
        for invalid in [json!(null), json!(-1), json!(101), json!("garbage")] {
            assert_eq!(parsed("claude", json!({"five_hour":{"utilization":invalid}}))["unavailable"], true);
        }
    }
    #[test]
    fn codex_windows_resets_credits_and_private_fields() {
        let facts = parsed(
            "openai-codex",
            json!({"email":"private","account_id":"private","plan_type":"pro","rate_limit":{"primary_window":{"used_percent":25,"limit_window_seconds":604800,"reset_at":1800000000,"reset_after_seconds":3600}},"additional_rate_limits":[{"limit_name":"Extra model","rate_limit":{"primary_window":{"used_percent":0,"limit_window_seconds":18000},"secondary_window":{"used_percent":100}}}],"credits":{"balance":"12.5","unlimited":false,"has_credits":true}}),
        );
        assert_eq!(facts["windows"].as_array().unwrap().len(), 3);
        assert_eq!(facts["windows"][0]["label"], "Codex · 7d");
        assert_eq!(facts["windows"][0]["reset"], 1800000000);
        assert_eq!(facts["windows"][2]["remaining"], 0.0);
        assert_eq!(facts["credits"]["remaining"], 12.5);
        assert!(!facts.to_string().contains("private"));
    }
    #[test]
    fn codex_reset_credits_keep_count_and_typed_dates_without_private_ids() {
        let facts = parsed(
            "openai-codex",
            json!({"rate_limit_reset_credits":{"available_count":2},"reset_credit_details":{"credits":[{"id":"private-id","title":"Bonus reset","status":"available","granted_at":1800000000,"expires_at":null},false,{"status":{},"granted_at":{}}]}}),
        );
        assert_eq!(facts["reset_count"], 2.0);
        assert_eq!(facts["reset_credits"].as_array().unwrap().len(), 1);
        assert_eq!(facts["reset_credits"][0]["granted"], 1800000000);
        assert_eq!(facts["reset_credits"][0]["expires"], Json::Null);
        assert!(!facts.to_string().contains("private-id"));
    }
    #[test]
    fn kimi_numeric_strings_partial_and_malformed_windows() {
        let facts = parsed(
            "kimi-coding",
            json!({"usage":{"limit":"100","remaining":"75","resetAt":"2026-09-08T00:00:00Z"},"limits":[false,{"name":"5h","detail":{"limit":20,"used":0}}]}),
        );
        assert_eq!(facts["windows"].as_array().unwrap().len(), 2);
        assert_eq!(facts["windows"][0]["used"], 25.0);
        assert_eq!(facts["windows"][1]["remaining"], 20.0);
        assert_eq!(parsed("kimi", json!({"limits":[{"detail":{"remaining":0}}]}))["windows"][0]["remaining"], 0.0);
        for provider in ["claude", "openai-codex", "kimi"] {
            assert_eq!(parsed(provider, Json::Null)["unavailable"], true);
        }
    }
    #[test]
    fn requests_keep_slots_and_kimi_composed_base_in_kernel() {
        let http = std::sync::Arc::new(crate::Http::new(std::sync::Arc::new(crate::Credentials::in_memory())).unwrap());
        let kimi = crate::provider::AnthropicMessages::from_preset(
            crate::presets::preset("kimi-coding").unwrap(),
            http.clone(),
        )
        .with_base_url("https://api.kimi.com/coding/v1");
        let req = request(&kimi).unwrap();
        assert_eq!(req.url, "https://api.kimi.com/coding/v1/usages");
        assert_eq!(req.credential.unwrap().slot, "kimi-coding");
        let codex =
            crate::provider::OpenAiResponses::from_preset(crate::presets::preset("openai-codex").unwrap(), http);
        let req = request(&codex).unwrap();
        assert_eq!(req.url, "https://chatgpt.com/backend-api/wham/usage");
        assert_eq!(req.credential.unwrap().account_header.as_deref(), Some("chatgpt-account-id"));
    }
}
