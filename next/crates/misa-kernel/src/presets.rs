//! The services a daemon can talk to, and what each one needs.
//!
//! There are two wire shapes in the world that matter here — OpenAI's chat completions
//! and Anthropic's messages — and a great many *services* that speak one of them. The
//! previous system drew exactly this line and got a great deal of coverage out of it:
//! a protocol is a parser and a request body, and a provider is a base url, a header,
//! a model list, and a couple of facts about its dialect. Groq is not a different
//! protocol from OpenAI; it is the same protocol at a different address, with
//! `max_completion_tokens` instead of `max_tokens` in one place and a different key page.
//!
//! # Why these facts and not more
//!
//! Every field here is something a request or a model list *does differently*, and
//! nothing here is a preference. `models_path` exists because Anthropic's version is
//! `/v1/models` and OpenAI's is `/models`. `reasoning_field` exists because DeepSeek
//! streams its reasoning in `reasoning_content` and OpenRouter in `reasoning`, and a
//! client that shows thinking has to be able to tell it from the answer.
//! `max_tokens_field` exists because the two names are both real and only one of them
//! is accepted by any given service.
//!
//! What is deliberately absent: prices, context windows, and which models exist. Those
//! are *facts about a model*, they change without a release, and the honest place for
//! them is the service's own model list — which is why [`crate::provider::discover`]
//! exists and why the session's catalog is enrichment on top of it rather than a
//! substitute.
//!
//! # Keys
//!
//! A preset names a *slot*, never a value: the id is the slot, and the daemon's
//! credential store is the only thing that ever holds the secret. That is the previous
//! system's rule, and it is what makes `misa-daemon login groq` and `--provider groq`
//! two halves of one idea.

use crate::http::Credential;

/// Where a key goes, for the services that want one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Auth {
    /// `Authorization: Bearer <key>`. Nearly everybody.
    Bearer,
    /// `x-api-key: <key>`. Anthropic, and the services that imitate it.
    ApiKey,
    /// No key at all, which is a model server on this machine.
    None,
}

/// How a service spells "think harder".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effort {
    /// `{"reasoning_effort": "high"}` — OpenAI's spelling, and the one most services
    /// that offer a choice have copied.
    Reasoning,
    /// `{"reasoning": {"effort": "high"}}` — OpenRouter's own shape, which it uses
    /// instead of the field it proxies.
    OpenRouter,
    /// `{"thinking": {"type": "enabled", "budget_tokens": n}}` — Anthropic, where an
    /// effort is a token budget rather than a word.
    Thinking,
    /// The service takes no such setting, and sending one is a 400.
    None,
}

/// One service, as its own documentation describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    /// The id a session names and the slot a credential lives in.
    pub id: &'static str,
    /// Which wire shape it speaks: an HTTP protocol or the Claude Code CLI.
    pub api: &'static str,
    /// Where the api lives. A server on this machine is a base url like any other.
    pub base_url: &'static str,
    /// Where that service lists its models, relative to the base url.
    pub models_path: &'static str,
    /// How a key travels, if one is needed at all.
    pub auth: Auth,
    /// Where a person gets a key. Shown by the login panel; never used by a request.
    pub keys_url: Option<&'static str>,
    /// The field a token budget goes in.
    pub max_tokens_field: &'static str,
    /// How this service spells a reasoning effort, if it takes one.
    pub effort: Effort,
}

impl Preset {
    /// The credential slot a request uses: the provider's own id.
    ///
    /// `None` for a service with no key, which is the difference between "this provider
    /// has no credential" and "this provider's credential is missing".
    pub fn slot(&self) -> Option<&'static str> {
        match self.auth {
            Auth::None => None,
            // Kimi exposes its coding API as provider `kimi`, while the account
            // flow is deliberately named `kimi-coding`. Provider identity and
            // credential identity are related, but they are not the same key.
            _ => Some(if self.id == "kimi" {
                "kimi-coding"
            } else {
                self.id
            }),
        }
    }

    /// The credential a request carries, by slot.
    pub fn credential(&self) -> Option<Credential> {
        let credential = match self.auth {
            Auth::Bearer => Credential::bearer(self.slot().expect("bearer presets have slots")),
            Auth::ApiKey => Credential::header(
                self.slot().expect("api-key presets have slots"),
                "x-api-key",
            ),
            Auth::None => return None,
        };
        // OpenAI's subscription backend wants the account a token belongs to as
        // well as the token, which the credential holds rather than the policy.
        Some(if self.id == "openai-codex" {
            credential.with_account_header("chatgpt-account-id")
        } else {
            credential
        })
    }

    /// Where this service's model list lives, as a full url.
    pub fn models_url(&self) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            self.models_path.trim_start_matches('/')
        )
    }

    /// Where a completion request goes.
    pub fn chat_url(&self) -> String {
        match self.api {
            "anthropic.messages" => format!("{}/v1/messages", self.base_url.trim_end_matches('/')),
            // The responses api is a path of its own, and its base url already ends at
            // the service's own prefix (`.../codex`), so nothing is appended to it.
            "openai.responses" => format!("{}/responses", self.base_url.trim_end_matches('/')),
            _ => format!("{}/chat/completions", self.base_url.trim_end_matches('/')),
        }
    }

    /// Whether the daemon has an adapter for this wire shape.
    pub fn implemented(&self) -> bool {
        matches!(
            self.api,
            "openai.chat" | "anthropic.messages" | "openai.responses" | "claude.cli"
        )
    }
}

/// Every service the shipped daemon knows by name.
///
/// The base urls are the ones each service publishes. A service that moves them is a
/// one-line change here, and a service that is not here is a `--base-url` away, because
/// the wire shape is what an adapter is and the address is not.
pub const PRESETS: &[Preset] = &[
    Preset {
        id: "openai",
        api: "openai.chat",
        base_url: "https://api.openai.com/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://platform.openai.com/api-keys"),
        // Only `max_completion_tokens` is accepted by OpenAI's reasoning models, and the
        // older field is deprecated there; every other service still wants `max_tokens`.
        max_tokens_field: "max_completion_tokens",
        effort: Effort::Reasoning,
    },
    Preset {
        id: "anthropic",
        api: "anthropic.messages",
        base_url: "https://api.anthropic.com",
        models_path: "v1/models",
        auth: Auth::ApiKey,
        keys_url: Some("https://console.anthropic.com/settings/keys"),
        max_tokens_field: "max_tokens",
        effort: Effort::Thinking,
    },
    Preset {
        id: "claude",
        api: "claude.cli",
        base_url: "",
        models_path: "",
        auth: Auth::None,
        keys_url: None,
        max_tokens_field: "max_tokens",
        effort: Effort::Thinking,
    },
    Preset {
        id: "deepseek",
        api: "openai.chat",
        base_url: "https://api.deepseek.com",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://platform.deepseek.com/api_keys"),
        max_tokens_field: "max_tokens",
        effort: Effort::Reasoning,
    },
    Preset {
        id: "openrouter",
        api: "openai.chat",
        base_url: "https://openrouter.ai/api/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://openrouter.ai/settings/keys"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::OpenRouter,
    },
    Preset {
        id: "groq",
        api: "openai.chat",
        base_url: "https://api.groq.com/openai/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://console.groq.com/keys"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::Reasoning,
    },
    Preset {
        id: "cerebras",
        api: "openai.chat",
        base_url: "https://api.cerebras.ai/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://cloud.cerebras.ai"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::None,
    },
    Preset {
        id: "together",
        api: "openai.chat",
        base_url: "https://api.together.ai/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://api.together.ai/settings/api-keys"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::Reasoning,
    },
    Preset {
        id: "fireworks",
        api: "openai.chat",
        base_url: "https://api.fireworks.ai/inference/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://fireworks.ai/account/api-keys"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::Reasoning,
    },
    Preset {
        id: "deepinfra",
        api: "openai.chat",
        base_url: "https://api.deepinfra.com/v1/openai",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://deepinfra.com/dash/api_keys"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::None,
    },
    Preset {
        id: "novita",
        api: "openai.chat",
        base_url: "https://api.novita.ai/openai/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://novita.ai/settings/key-management"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::Reasoning,
    },
    Preset {
        id: "siliconflow",
        api: "openai.chat",
        base_url: "https://api.siliconflow.com/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://cloud.siliconflow.com/account/ak"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::Reasoning,
    },
    Preset {
        id: "venice",
        api: "openai.chat",
        base_url: "https://api.venice.ai/api/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://venice.ai/settings/api"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::None,
    },
    Preset {
        id: "nvidia",
        api: "openai.chat",
        base_url: "https://integrate.api.nvidia.com/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://build.nvidia.com"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::Reasoning,
    },
    Preset {
        id: "huggingface",
        api: "openai.chat",
        base_url: "https://router.huggingface.co/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://huggingface.co/settings/tokens"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::None,
    },
    Preset {
        id: "mistral",
        api: "openai.chat",
        base_url: "https://api.mistral.ai/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://console.mistral.ai/api-keys"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::None,
    },
    Preset {
        id: "moonshot",
        api: "openai.chat",
        base_url: "https://api.moonshot.ai/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://platform.moonshot.ai/console/api-keys"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::Reasoning,
    },
    Preset {
        id: "kimi",
        api: "anthropic.messages",
        // The Anthropic adapter appends `/v1/messages`; keep the common base
        // at `/coding` so the effective Kimi endpoint remains `/coding/v1`.
        base_url: "https://api.kimi.ai/coding",
        models_path: "v1/models",
        auth: Auth::Bearer,
        keys_url: Some("https://auth.kimi.ai/api/oauth/device_authorization"),
        max_tokens_field: "max_tokens",
        effort: Effort::Thinking,
    },
    Preset {
        id: "xai",
        api: "openai.chat",
        base_url: "https://api.x.ai/v1",
        models_path: "models",
        auth: Auth::Bearer,
        keys_url: Some("https://console.x.ai"),
        max_tokens_field: "max_completion_tokens",
        effort: Effort::Reasoning,
    },
    // The two subscription services. Neither takes an API key at all: a token
    // comes from a device flow, which is what [`oauth`] describes, and the
    // credential slot is where the token — and its refresh token — live.
    Preset {
        id: "openai-codex",
        api: "openai.responses",
        base_url: "https://chatgpt.com/backend-api/codex",
        models_path: "models?client_version=0.153.4",
        auth: Auth::Bearer,
        keys_url: Some("https://auth.openai.com/codex/device"),
        max_tokens_field: "max_output_tokens",
        effort: Effort::Reasoning,
    },
    // `kimi-coding` is the credential slot for the `kimi` provider above, not a
    // second model provider. Its OAuth flow is declared below.
];

/// The device flow for a service that hands out tokens rather than keys.
///
/// The client ids are the ones the services' own command-line tools are
/// registered under, which is what makes a daemon able to authorize an account
/// without anybody registering an application first. The endpoints are the ones
/// those tools use, and both are here rather than in a preset because a preset is
/// about a *request* and this is about getting the credential a request needs.
pub fn oauth(id: &str) -> Option<crate::oauth::Flow> {
    flows()
        .into_iter()
        .find(|(provider, _)| *provider == id)
        .map(|(_, flow)| flow)
}

/// Every device flow, as the provider that names it and the flow itself.
///
/// One list, so that the daemon which authorizes a provider and the session that offers to
/// cannot disagree about which providers those are. A composition passes this and may
/// replace any of them ([`crate::Daemon::with_flow`]), which is what makes a flow written
/// against a service on the internet testable against a server on this machine.
pub fn flows() -> Vec<(&'static str, crate::oauth::Flow)> {
    use crate::oauth::Kind;
    vec![
        (
            "openai-codex",
            crate::oauth::Flow {
                kind: Kind::OpenAi,
                client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
                authorization_url: "https://auth.openai.com/api/accounts/deviceauth/usercode",
                token_url: "https://auth.openai.com/oauth/token",
                verification_url: "https://auth.openai.com/codex/device",
            },
        ),
        (
            "kimi-coding",
            crate::oauth::Flow {
                kind: Kind::Rfc8628,
                client_id: "17e5f671-d194-4dfb-9706-5516cb48c098",
                authorization_url: "https://auth.kimi.ai/api/oauth/device_authorization",
                token_url: "https://auth.kimi.ai/api/oauth/token",
                // RFC 8628 answers with the place to type the code; this is the
                // fallback for a service that does not.
                verification_url: "https://auth.kimi.ai/api/oauth/device_authorization",
            },
        ),
    ]
}

/// The preset with this id, if it is one the daemon knows by name.
pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

/// Every id, for a diagnostic or a picker.
pub fn ids() -> Vec<&'static str> {
    PRESETS.iter().map(|preset| preset.id).collect()
}

/// Credential slots the standard composition can log in to.
///
/// Search keys are credential slots too. They are not model presets, but they
/// are part of the same `/login` vocabulary in the reference client and must
/// not disappear merely because they do not have a model catalogue.
pub fn auth_ids() -> Vec<&'static str> {
    let mut ids = PRESETS
        .iter()
        .filter(|preset| preset.auth != Auth::None || preset.api == "claude.cli")
        .filter_map(|preset| {
            // Claude Code deliberately has no secret slot: authentication is a
            // local CLI handoff. It is still an auth choice, though, and must be
            // present in the same provider picker as token/device-flow entries.
            preset
                .slot()
                .or((preset.api == "claude.cli").then_some(preset.id))
        })
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids.extend(["brave", "tavily"]);
    ids.sort_unstable();
    ids
}

/// Resolve the provider that owns a credential slot.
pub fn preset_for_slot(slot: &str) -> Option<&'static Preset> {
    PRESETS
        .iter()
        .find(|preset| preset.slot() == Some(slot) || preset.id == slot)
}

/// The public provider vocabulary, for a daemon that was asked to list it.
///
/// Provider ids are deliberately the output. They are stable values used in
/// commands, credentials, and model qualification; prose labels here only
/// create a second vocabulary that the picker cannot accept verbatim.
pub fn describe() -> String {
    PRESETS
        .iter()
        .map(|preset| preset.id)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_says_what_it_is_and_where_it_lives() {
        for preset in PRESETS {
            assert!(!preset.id.is_empty());
            // Dashes and underscores are part of the vocabulary an id names: the
            // service is `openai` and the subscription provider is `openai-codex`,
            // and the slot is the same string either way.
            assert!(
                preset.id.chars().all(|ch| ch.is_ascii_lowercase()
                    || ch.is_ascii_digit()
                    || ch == '-'
                    || ch == '_'),
                "`{}` is not an id a command line would take",
                preset.id
            );
            match preset.api {
                // A service with a wire shape needs an address to reach it, and one with a
                // key needs a page that hands one out, because a person has to be able to
                // find it without reading this file.
                "openai.chat" | "anthropic.messages" | "openai.responses" => {
                    assert!(
                        preset.base_url.starts_with("http"),
                        "`{}` has no address",
                        preset.id
                    );
                    assert!(
                        !preset.models_path.is_empty(),
                        "`{}` lists no models",
                        preset.id
                    );
                    if preset.auth != Auth::None {
                        assert!(
                            preset.keys_url.is_some(),
                            "`{}` says where to get a key",
                            preset.id
                        );
                    }
                }
                "claude.cli" => assert!(preset.base_url.is_empty()),
                other => panic!(
                    "`{}` speaks `{other}`, which is not a wire shape",
                    preset.id
                ),
            }
        }
    }

    #[test]
    fn a_key_is_never_a_value_here_only_a_slot() {
        // The rule the whole credential design rests on: policy names a slot, and the daemon
        // injects the value. A preset that could carry a key would be a preset with a secret
        // in a source file.
        for preset in PRESETS {
            assert!(preset.slot().is_none_or(|slot| !slot.is_empty()));
            let described = format!("{preset:?}");
            assert!(!described.contains("sk-"), "{described}");
        }
        assert_eq!(preset("openai").unwrap().credential().is_some(), true);
        assert_eq!(preset("claude").unwrap().credential().is_none(), true);
    }

    #[test]
    fn the_urls_each_service_uses_are_built_from_its_own_shape() {
        let openai = preset("openai").unwrap();
        assert_eq!(
            openai.chat_url(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(openai.models_url(), "https://api.openai.com/v1/models");
        let anthropic = preset("anthropic").unwrap();
        assert_eq!(
            anthropic.chat_url(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            anthropic.models_url(),
            "https://api.anthropic.com/v1/models"
        );
        let deepseek = preset("deepseek").unwrap();
        assert_eq!(
            deepseek.chat_url(),
            "https://api.deepseek.com/chat/completions"
        );
        assert_eq!(deepseek.models_url(), "https://api.deepseek.com/models");
        let kimi = preset("kimi").unwrap();
        assert_eq!(kimi.chat_url(), "https://api.kimi.ai/coding/v1/messages");
        assert_eq!(kimi.models_url(), "https://api.kimi.ai/coding/v1/models");
        let codex = preset("openai-codex").unwrap();
        assert_eq!(
            codex.models_url(),
            "https://chatgpt.com/backend-api/codex/models?client_version=0.153.4"
        );
        assert_eq!(preset("nope"), None);
        assert!(ids().contains(&"deepseek"));
    }
    #[test]
    fn every_service_this_daemon_knows_has_an_adapter_or_says_so() {
        // A preset a daemon cannot serve is a picker that offers a provider that
        // always fails, which is worse than not offering it.
        for preset in PRESETS {
            assert!(
                preset.implemented(),
                "`{}` speaks `{}`, which no adapter speaks",
                preset.id,
                preset.api
            );
        }
    }

    #[test]
    fn openai_compatible_presets_use_the_reference_output_limit_field() {
        for id in [
            "openai",
            "openrouter",
            "groq",
            "cerebras",
            "together",
            "fireworks",
            "deepinfra",
            "novita",
            "siliconflow",
            "venice",
            "nvidia",
            "huggingface",
            "mistral",
            "moonshot",
            "xai",
        ] {
            assert_eq!(
                preset(id).unwrap().max_tokens_field,
                "max_completion_tokens",
                "{id}"
            );
        }
        assert_eq!(preset("deepseek").unwrap().max_tokens_field, "max_tokens");
    }

    #[test]
    fn the_subscription_services_have_a_device_flow_and_the_key_services_do_not() {
        assert!(oauth("openai-codex").is_some());
        assert!(oauth("kimi-coding").is_some());
        assert!(
            oauth("openai").is_none(),
            "a service with a key page has no device flow"
        );
        assert!(ids().contains(&"openai-codex"));
        assert!(ids().contains(&"kimi"));
        assert!(auth_ids().contains(&"kimi-coding"));
        assert!(auth_ids().contains(&"claude"));
        assert!(auth_ids().contains(&"brave"));
        assert!(auth_ids().contains(&"tavily"));
        assert!(!ids().contains(&"scripted"));
        assert!(!auth_ids().contains(&"scripted"));
    }
}
