//! Secrets, and the rule that they never cross into policy.
//!
//! A policy names a *slot* — `anthropic`, `openai`, `search.brave` — and the kernel
//! turns that into a header at request time. A credential's bytes are therefore never
//! in a database, a view, a log, or an event, and a plugin cannot exfiltrate what it
//! was never given. This is the previous system's rule ("`http/request` injects
//! credentials by ID inside Zig, so secret bytes never cross into Fennel policy"),
//! kept because it is the whole of what makes an effect boundary a security boundary.
//!
//! Storage is one JSON file, mode 0600, in the state directory. It is not a keyring
//! and does not pretend to be: it is the smallest thing that is honest about being
//! plaintext on disk, and it is exactly what the previous system did with the
//! preference document's neighbours.
//!
//! # Two kinds of credential, one slot
//!
//! A key is a string that never expires. An OAuth credential is an access token with
//! a deadline and a refresh token with none, and the *deadline* is what makes it
//! different: something has to notice it and renew it. The slot is the same either
//! way, so a policy names one thing and the store decides whether renewal is part of
//! using it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Secret {
    pub account: String,
    pub value: String,
}

/// An OAuth credential: what makes a request now, and what makes one later.
#[derive(Clone, Debug, PartialEq)]
pub struct OAuth {
    pub account: String,
    pub access: String,
    /// The thing worth keeping. An empty refresh token is a credential that will
    /// stop working and cannot be renewed, which is a fact worth storing honestly.
    pub refresh: String,
    pub expires_ms: i64,
    /// Where to renew it, and as which client. The service's own answer, kept
    /// because a token endpoint is not something a policy should have to know.
    pub token_url: String,
    pub client_id: String,
}

impl OAuth {
    /// The credential as the store holds it: the account is shared, and the
    /// access token is what a request carries.
    pub fn secret(&self) -> Secret {
        Secret {
            account: self.account.clone(),
            value: self.access.clone(),
        }
    }
}

pub struct Credentials {
    path: Option<PathBuf>,
    /// Loaded once and kept, so a request does not read a file.
    secrets: std::sync::Mutex<BTreeMap<(String, String), Secret>>,
    oauth: std::sync::Mutex<BTreeMap<(String, String), OAuth>>,
    /// The account selected for each provider. Selection is capability state, not
    /// presentation state: every request resolves the active account here.
    active: std::sync::Mutex<BTreeMap<String, String>>,
}

impl Credentials {
    pub fn in_memory() -> Credentials {
        Credentials {
            path: None,
            secrets: std::sync::Mutex::new(BTreeMap::new()),
            oauth: std::sync::Mutex::new(BTreeMap::new()),
            active: std::sync::Mutex::new(BTreeMap::new()),
        }
    }

    /// Open the store at `path`, reading whatever is there.
    pub fn at(path: &Path) -> Result<Credentials, String> {
        let (secrets, oauth, active) = match std::fs::read_to_string(path) {
            Ok(text) => parse(&text)?,
            // A missing file is a store that has never been written to, not an error:
            // most sessions have no credentials at all.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                (BTreeMap::new(), BTreeMap::new(), BTreeMap::new())
            }
            Err(err) => return Err(format!("could not read {}: {err}", path.display())),
        };
        Ok(Credentials {
            path: Some(path.to_path_buf()),
            secrets: std::sync::Mutex::new(secrets),
            oauth: std::sync::Mutex::new(oauth),
            active: std::sync::Mutex::new(active),
        })
    }

    /// The reference store's location: `$MISA_AUTH_FILE`, else the state directory.
    ///
    /// `MISA_CREDENTIALS` remains an explicit compatibility alias for early next
    /// builds, but the default must be the reference file so an already-authenticated
    /// provider is visible after moving between the two implementations.
    pub fn default_path() -> PathBuf {
        for variable in ["MISA_AUTH_FILE", "MISA_CREDENTIALS"] {
            if let Ok(explicit) = std::env::var(variable)
                && !explicit.is_empty()
            {
                return PathBuf::from(explicit);
            }
        }
        let state = std::env::var("XDG_STATE_HOME")
            .ok()
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|home| PathBuf::from(home).join(".local/state"))
            })
            .unwrap_or_else(|| PathBuf::from("."));
        state.join("misa/auth.json")
    }

    /// The secret for a slot, if there is one.
    ///
    /// The only way out of this type, and it is used by the HTTP capability — not by
    /// anything a policy can call. An OAuth slot answers with its access token, so
    /// the capability that injects a header does not have to know which kind it is.
    pub fn secret(&self, slot: &str) -> Option<String> {
        let account = self.active_account(slot)?;
        if let Some(secret) = self
            .secrets
            .lock()
            .ok()?
            .get(&(slot.to_string(), account.clone()))
        {
            return Some(secret.value.clone());
        }
        self.oauth
            .lock()
            .ok()?
            .get(&(slot.to_string(), account))
            .map(|oauth| oauth.access.clone())
    }

    /// The OAuth credential for a slot, for the code that renews it.
    pub fn oauth(&self, slot: &str) -> Option<OAuth> {
        let account = self.active_account(slot)?;
        self.oauth
            .lock()
            .ok()?
            .get(&(slot.to_string(), account))
            .cloned()
    }

    pub fn has(&self, slot: &str) -> bool {
        self.active_account(slot).is_some_and(|account| {
            self.secrets
                .lock()
                .map(|secrets| secrets.contains_key(&(slot.to_string(), account.clone())))
                .unwrap_or(false)
                || self
                    .oauth
                    .lock()
                    .map(|oauth| oauth.contains_key(&(slot.to_string(), account)))
                    .unwrap_or(false)
        })
    }

    fn active_account(&self, slot: &str) -> Option<String> {
        let selected = self.active.lock().ok()?.get(slot).cloned();
        let exists = |account: &str| {
            self.secrets
                .lock()
                .map(|secrets| secrets.contains_key(&(slot.to_string(), account.to_string())))
                .unwrap_or(false)
                || self
                    .oauth
                    .lock()
                    .map(|oauth| oauth.contains_key(&(slot.to_string(), account.to_string())))
                    .unwrap_or(false)
        };
        if let Some(account) = selected.filter(|account| exists(account)) {
            return Some(account);
        }
        // An active pointer can outlive an account removed by another process or an
        // older file migration. Never let that stale pointer hide a valid credential.
        self.secrets
            .lock()
            .ok()?
            .keys()
            .find(|(provider, _)| provider == slot)
            .map(|(_, account)| account.clone())
            .or_else(|| {
                self.oauth
                    .lock()
                    .ok()?
                    .keys()
                    .find(|(provider, _)| provider == slot)
                    .map(|(_, account)| account.clone())
            })
    }

    /// The selected account, without exposing a token.
    pub fn account(&self, slot: &str) -> Option<String> {
        self.active_account(slot)
    }

    /// Every named account held by the provider, with the selected one first.
    pub fn accounts(&self, slot: &str) -> Vec<String> {
        let mut names = self
            .secrets
            .lock()
            .map(|secrets| {
                secrets
                    .keys()
                    .filter(|(provider, _)| provider == slot)
                    .map(|(_, account)| account.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Ok(oauth) = self.oauth.lock() {
            names.extend(
                oauth
                    .keys()
                    .filter(|(provider, _)| provider == slot)
                    .map(|(_, account)| account.clone()),
            );
        }
        names.sort();
        names.dedup();
        if let Some(active) = self.active_account(slot)
            && let Some(index) = names.iter().position(|name| name == &active)
        {
            names.remove(index);
            names.insert(0, active);
        }
        names
    }

    pub fn select(&self, slot: &str, account: &str) -> Result<bool, String> {
        let account = if account.is_empty() {
            "default"
        } else {
            account
        };
        if !valid_account(account) {
            return Err("a credential needs a valid account name".into());
        }
        let exists = self
            .secrets
            .lock()
            .map(|secrets| secrets.contains_key(&(slot.to_string(), account.to_string())))
            .unwrap_or(false)
            || self
                .oauth
                .lock()
                .map(|oauth| oauth.contains_key(&(slot.to_string(), account.to_string())))
                .unwrap_or(false);
        if !exists {
            return Ok(false);
        }
        {
            self.active
                .lock()
                .map_err(|_| "the credential store is poisoned".to_string())?
                .insert(slot.to_string(), account.to_string());
        }
        self.save()?;
        Ok(true)
    }

    /// Slots and their accounts. Never their values.
    pub fn slots(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .secrets
            .lock()
            .map(|secrets| secrets.keys().cloned().collect())
            .unwrap_or_default();
        if let Ok(oauth) = self.oauth.lock() {
            out.extend(oauth.keys().cloned());
        }
        out.sort();
        out.dedup();
        out.sort();
        out
    }

    pub fn set(&self, slot: &str, account: &str, value: &str) -> Result<(), String> {
        if slot.is_empty() {
            return Err("a credential needs a slot".into());
        }
        let account = if account.is_empty() {
            "default"
        } else {
            account
        };
        if !valid_account(account) {
            return Err("a credential needs a valid account name".into());
        }
        if value.trim().is_empty() {
            return Err("a credential needs a value".into());
        }
        {
            // A key replaces a token, and a token replaces a key: one slot names
            // one way of authenticating, and keeping both would be a slot with
            // two answers.
            let mut oauth = self
                .oauth
                .lock()
                .map_err(|_| "the credential store is poisoned".to_string())?;
            oauth.retain(|(provider, name), _| !(provider == slot && name == account));
            let mut secrets = self
                .secrets
                .lock()
                .map_err(|_| "the credential store is poisoned".to_string())?;
            secrets.insert(
                (slot.to_string(), account.to_string()),
                Secret {
                    account: account.to_string(),
                    value: value.trim().to_string(),
                },
            );
        }
        {
            self.active
                .lock()
                .map_err(|_| "the credential store is poisoned".to_string())?
                .insert(slot.to_string(), account.to_string());
        }
        self.save()
    }

    /// Store an OAuth credential, replacing whatever the slot held.
    pub fn set_oauth(&self, slot: &str, oauth: OAuth) -> Result<(), String> {
        let account = if oauth.account.is_empty() {
            "default".into()
        } else {
            oauth.account.clone()
        };
        self.set_oauth_for(slot, &account, oauth)
    }

    /// Store an OAuth credential under a named account. `OAuth::account` remains
    /// the provider-issued account id used by services such as OpenAI.
    pub fn set_oauth_for(&self, slot: &str, account: &str, oauth: OAuth) -> Result<(), String> {
        if slot.is_empty() {
            return Err("a credential needs a slot".into());
        }
        if !valid_account(account) {
            return Err("a credential needs a valid account name".into());
        }
        if oauth.access.trim().is_empty() {
            return Err("an oauth credential needs an access token".into());
        }
        {
            let mut secrets = self
                .secrets
                .lock()
                .map_err(|_| "the credential store is poisoned".to_string())?;
            secrets.remove(&(slot.to_string(), account.to_string()));
            let mut stored = self
                .oauth
                .lock()
                .map_err(|_| "the credential store is poisoned".to_string())?;
            stored.insert((slot.to_string(), account.to_string()), oauth);
        }
        {
            self.active
                .lock()
                .map_err(|_| "the credential store is poisoned".to_string())?
                .insert(slot.to_string(), account.to_string());
        }
        self.save()
    }

    /// Keep the same credential with a fresh access token.
    ///
    /// Separate from [`Credentials::set_oauth`] because renewal replaces exactly
    /// two fields and nothing else: an account, a token endpoint, and a client id
    /// came from an authorization a person gave, and a refresh did not change them.
    pub fn renew(
        &self,
        slot: &str,
        access: &str,
        refresh: &str,
        expires_ms: i64,
    ) -> Result<bool, String> {
        let account = self.active_account(slot).unwrap_or_default();
        let mut stored = self
            .oauth
            .lock()
            .map_err(|_| "the credential store is poisoned".to_string())?;
        let Some(oauth) = stored.get_mut(&(slot.to_string(), account)) else {
            return Ok(false);
        };
        oauth.access = access.to_string();
        if !refresh.is_empty() {
            oauth.refresh = refresh.to_string();
        }
        oauth.expires_ms = expires_ms;
        drop(stored);
        self.save()?;
        Ok(true)
    }

    pub fn delete(&self, slot: &str) -> Result<bool, String> {
        let Some(account) = self.active_account(slot) else {
            return Ok(false);
        };
        self.delete_account(slot, &account)
    }

    pub fn delete_account(&self, slot: &str, account: &str) -> Result<bool, String> {
        let removed = {
            let mut secrets = self
                .secrets
                .lock()
                .map_err(|_| "the credential store is poisoned".to_string())?;
            let removed = secrets
                .remove(&(slot.to_string(), account.to_string()))
                .is_some();
            let mut oauth = self
                .oauth
                .lock()
                .map_err(|_| "the credential store is poisoned".to_string())?;
            removed
                || oauth
                    .remove(&(slot.to_string(), account.to_string()))
                    .is_some()
        };
        if removed {
            let replacement = self.accounts(slot).into_iter().find(|name| name != account);
            {
                let mut active = self
                    .active
                    .lock()
                    .map_err(|_| "the credential store is poisoned".to_string())?;
                if replacement.is_some() {
                    active.insert(slot.to_string(), replacement.unwrap());
                } else {
                    active.remove(slot);
                }
            }
            self.save()?;
        }
        Ok(removed)
    }

    /// Write the file, private, replacing it as a whole.
    ///
    /// A rename rather than a truncate, so a crash cannot leave half a store, and 0600
    /// before the bytes are in it rather than after.
    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            let create_directory = !parent.exists();
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            if create_directory {
                set_private_directory(parent)?;
            }
        }
        let text = {
            let secrets = self
                .secrets
                .lock()
                .map_err(|_| "the credential store is poisoned".to_string())?;
            let oauth = self
                .oauth
                .lock()
                .map_err(|_| "the credential store is poisoned".to_string())?;
            let active = self
                .active
                .lock()
                .map_err(|_| "the credential store is poisoned".to_string())?;
            render(&secrets, &oauth, &active)
        };
        let temporary = path.with_extension("json.part");
        std::fs::write(&temporary, text).map_err(|err| err.to_string())?;
        set_private(&temporary)?;
        std::fs::rename(&temporary, path).map_err(|err| err.to_string())?;
        Ok(())
    }
}

impl Default for Credentials {
    fn default() -> Self {
        Credentials::in_memory()
    }
}

#[cfg(unix)]
fn set_private(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|err| err.to_string())
}

#[cfg(not(unix))]
fn set_private(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|err| err.to_string())
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// The file format: one JSON object, slot to `{account, value}`, and — for a slot
/// that holds a token rather than a key — `{account, value, refresh, expires_ms,
/// token_url, client_id}`. A `refresh` field is what makes it a token, so a file
/// written before OAuth existed still reads.
fn render(
    secrets: &BTreeMap<(String, String), Secret>,
    oauth: &BTreeMap<(String, String), OAuth>,
    active: &BTreeMap<String, String>,
) -> String {
    let mut providers = BTreeMap::<String, Vec<(String, String)>>::new();
    for ((slot, account), secret) in secrets {
        providers.entry(slot.clone()).or_default().push((
            account.clone(),
            format!(
                "{{\"type\": \"api_key\", \"access\": \"{}\"}}",
                escape(&secret.value),
            ),
        ));
    }
    for ((slot, account), token) in oauth {
        providers.entry(slot.clone()).or_default().push((account.clone(), format!(
            "{{\"type\": \"oauth\", \"access\": \"{}\", \"refresh\": \"{}\", \"expires\": {}, \"expires_ms\": {}, \"account_id\": \"{}\", \"token_url\": \"{}\", \"client_id\": \"{}\"}}",
            escape(&token.access),
            escape(&token.refresh),
            token.expires_ms / 1_000,
            token.expires_ms,
            escape(&token.account),
            escape(&token.token_url),
            escape(&token.client_id),
        )));
    }
    let mut rows: Vec<(String, String)> = Vec::new();
    for (slot, mut accounts) in providers {
        accounts.sort_by(|left, right| left.0.cmp(&right.0));
        let selected = active
            .get(&slot)
            .cloned()
            .or_else(|| accounts.first().map(|(name, _)| name.clone()))
            .unwrap_or_default();
        let accounts_json = accounts
            .into_iter()
            .map(|(name, credential)| format!("\"{}\": {credential}", escape(&name)))
            .collect::<Vec<_>>()
            .join(", ");
        rows.push((
            slot,
            format!(
                "{{\"active\": \"{}\", \"accounts\": {{{accounts_json}}}}}",
                escape(&selected)
            ),
        ));
    }
    rows.sort();
    let mut out = String::from("{\n");
    for (index, (slot, row)) in rows.iter().enumerate() {
        if index > 0 {
            out.push_str(",\n");
        }
        out.push_str(&format!("  \"{}\": {row}", escape(slot)));
    }
    out.push_str("\n}\n");
    out
}

fn parse(
    text: &str,
) -> Result<
    (
        BTreeMap<(String, String), Secret>,
        BTreeMap<(String, String), OAuth>,
        BTreeMap<String, String>,
    ),
    String,
> {
    let parsed: serde_json::Value = serde_json::from_str(text)
        .map_err(|err| format!("the credential store is not readable: {err}"))?;
    let mut secrets = BTreeMap::new();
    let mut oauth = BTreeMap::new();
    let mut active = BTreeMap::new();
    let Some(object) = parsed.as_object() else {
        return Err("the credential store should be an object".into());
    };
    for (slot, entry) in object {
        if let Some(accounts) = entry
            .get("accounts")
            .and_then(|accounts| accounts.as_object())
        {
            if let Some(name) = entry.get("active").and_then(|name| name.as_str()) {
                active.insert(slot.clone(), name.to_string());
            }
            for (name, credential) in accounts {
                parse_credential(&mut secrets, &mut oauth, slot, name, credential)?;
            }
        } else {
            // Migrate the next client's single-account file, and the old reference's
            // pre-account file, without requiring a one-time conversion command.
            let name = entry
                .get("account")
                .and_then(|account| account.as_str())
                .filter(|name| valid_account(name))
                .unwrap_or("default");
            active.insert(slot.clone(), name.to_string());
            parse_credential(&mut secrets, &mut oauth, slot, name, entry)?;
        }
    }
    Ok((secrets, oauth, active))
}

fn parse_credential(
    secrets: &mut BTreeMap<(String, String), Secret>,
    oauth: &mut BTreeMap<(String, String), OAuth>,
    slot: &str,
    name: &str,
    entry: &serde_json::Value,
) -> Result<(), String> {
    let access = entry
        .get("value")
        .and_then(|value| value.as_str())
        .or_else(|| entry.get("access").and_then(|value| value.as_str()))
        .or_else(|| entry.as_str());
    let Some(access) = access else {
        return Ok(());
    };
    if let Some(refresh) = entry.get("refresh").and_then(|refresh| refresh.as_str()) {
        let expires_ms = entry
            .get("expires_ms")
            .and_then(|at| at.as_i64())
            .or_else(|| {
                entry
                    .get("expires")
                    .and_then(|at| at.as_i64())
                    .map(|at| if at < 10_000_000_000 { at * 1_000 } else { at })
            })
            .unwrap_or(0);
        oauth.insert(
            (slot.to_string(), name.to_string()),
            OAuth {
                account: entry
                    .get("account_id")
                    .and_then(|account| account.as_str())
                    .or_else(|| entry.get("account").and_then(|account| account.as_str()))
                    .unwrap_or_default()
                    .into(),
                access: access.into(),
                refresh: refresh.into(),
                expires_ms,
                token_url: entry
                    .get("token_url")
                    .and_then(|url| url.as_str())
                    .unwrap_or_default()
                    .into(),
                client_id: entry
                    .get("client_id")
                    .and_then(|id| id.as_str())
                    .unwrap_or_default()
                    .into(),
            },
        );
    } else {
        secrets.insert(
            (slot.to_string(), name.to_string()),
            Secret {
                account: name.into(),
                value: access.into(),
            },
        );
    }
    Ok(())
}

fn valid_account(account: &str) -> bool {
    !account.is_empty()
        && account.len() <= 64
        && account
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if (other as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", other as u32)),
            other => out.push(other),
        }
    }
    out
}

/// Whether a value looks like it was meant to be a secret, for a diagnostic.
///
/// Not a validator: a token's shape is the provider's business. This says "that looks
/// like a path, or a word" so a mistyped argument is caught before it is stored.
pub fn looks_like_a_secret(value: &str) -> bool {
    let value = value.trim();
    value.len() >= 12 && !value.contains('/') && !value.contains(' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_round_trips_and_never_appears_in_the_listing() {
        let store = Credentials::in_memory();
        store
            .set("anthropic", "work", "sk-ant-secret-value")
            .unwrap();
        assert!(store.has("anthropic"));
        assert_eq!(
            store.secret("anthropic").as_deref(),
            Some("sk-ant-secret-value")
        );
        // The listing is what a diagnostic may print, so it carries no value.
        assert_eq!(
            store.slots(),
            vec![("anthropic".to_string(), "work".to_string())]
        );
        assert!(store.delete("anthropic").unwrap());
        assert!(!store.has("anthropic"));
        assert!(!store.delete("anthropic").unwrap());
    }

    #[test]
    fn an_empty_value_or_slot_is_refused() {
        let store = Credentials::in_memory();
        assert!(store.set("", "a", "value").is_err());
        assert!(store.set("slot", "a", "   ").is_err());
    }

    #[test]
    fn an_oauth_credential_is_a_slot_like_any_other_and_answers_with_its_access_token() {
        // A policy names a slot; whether the thing behind it is a key or a token
        // is the store's business, which is what lets the HTTP capability inject
        // either without knowing which.
        let store = Credentials::in_memory();
        store
            .set_oauth(
                "openai-codex",
                OAuth {
                    account: "acct".into(),
                    access: "an-access".into(),
                    refresh: "a-refresh".into(),
                    expires_ms: 1_000,
                    token_url: "https://auth.example/token".into(),
                    client_id: "a-client".into(),
                },
            )
            .unwrap();
        assert!(store.has("openai-codex"));
        assert_eq!(store.secret("openai-codex").as_deref(), Some("an-access"));
        assert_eq!(store.oauth("openai-codex").unwrap().refresh, "a-refresh");
        assert_eq!(
            store.slots(),
            vec![("openai-codex".to_string(), "acct".to_string())]
        );
        // And setting a key over it is one slot with one answer, not two.
        store
            .set("openai-codex", "acct", "a-key-value-here")
            .unwrap();
        assert_eq!(
            store.secret("openai-codex").as_deref(),
            Some("a-key-value-here")
        );
        assert!(store.oauth("openai-codex").is_none());
    }

    #[test]
    fn a_stored_secret_survives_a_reopen_and_is_private_on_disk() {
        let path = std::env::temp_dir().join(format!("misa-cred-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let store = Credentials::at(&path).unwrap();
            store.set("openai", "personal", "sk-openai-value").unwrap();
        }
        let reopened = Credentials::at(&path).unwrap();
        assert_eq!(
            reopened.secret("openai").as_deref(),
            Some("sk-openai-value")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the credential file is readable by others");
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_oauth_credential_survives_a_reopen_with_its_refresh_token() {
        let path =
            std::env::temp_dir().join(format!("misa-cred-oauth-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let store = Credentials::at(&path).unwrap();
            store
                .set_oauth(
                    "kimi-coding",
                    OAuth {
                        account: "global".into(),
                        access: "an-access".into(),
                        refresh: "a-refresh".into(),
                        expires_ms: 42,
                        token_url: "https://auth.kimi.ai/api/oauth/token".into(),
                        client_id: "a-client".into(),
                    },
                )
                .unwrap();
        }
        let reopened = Credentials::at(&path).unwrap();
        let oauth = reopened.oauth("kimi-coding").expect("a token, not a key");
        assert_eq!(oauth.access, "an-access");
        assert_eq!(oauth.refresh, "a-refresh");
        assert_eq!(oauth.expires_ms, 42);
        assert_eq!(oauth.token_url, "https://auth.kimi.ai/api/oauth/token");
        assert_eq!(oauth.client_id, "a-client");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_is_an_empty_store_and_a_corrupt_one_is_an_error() {
        let path =
            std::env::temp_dir().join(format!("misa-cred-missing-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(Credentials::at(&path).unwrap().slots().is_empty());
        std::fs::write(&path, "not json at all").unwrap();
        assert!(
            Credentials::at(&path).is_err(),
            "a corrupt store was treated as empty"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_value_with_quotes_or_newlines_does_not_break_the_file() {
        let store = Credentials::in_memory();
        store.set("odd", "a-b", "line\n\"two\"").unwrap();
        let rendered = render(
            &store.secrets.lock().unwrap(),
            &BTreeMap::new(),
            &store.active.lock().unwrap(),
        );
        let (reparsed, _, _) = parse(&rendered).unwrap();
        assert_eq!(
            reparsed[&(String::from("odd"), String::from("a-b"))].value,
            "line\n\"two\""
        );
        assert_eq!(
            reparsed[&(String::from("odd"), String::from("a-b"))].account,
            "a-b"
        );
    }

    #[test]
    fn a_likely_mistake_is_caught_before_it_is_stored() {
        assert!(looks_like_a_secret("sk-ant-0123456789"));
        assert!(!looks_like_a_secret("/home/me/token.txt"));
        assert!(!looks_like_a_secret("short"));
    }

    #[test]
    fn named_accounts_are_selected_without_replacing_each_other() {
        let store = Credentials::in_memory();
        store.set("openai", "personal", "personal-token").unwrap();
        store.set("openai", "work", "work-token").unwrap();
        assert_eq!(store.accounts("openai"), vec!["work", "personal"]);
        assert_eq!(store.secret("openai").as_deref(), Some("work-token"));
        assert!(store.select("openai", "personal").unwrap());
        assert_eq!(store.secret("openai").as_deref(), Some("personal-token"));
        assert!(store.delete_account("openai", "personal").unwrap());
        assert_eq!(store.secret("openai").as_deref(), Some("work-token"));
        assert_eq!(store.slots(), vec![("openai".into(), "work".into())]);
    }

    #[test]
    fn old_single_account_and_reference_account_files_are_read() {
        let (secrets, _, active) = parse(r#"{"openai":{"account":"default","value":"key"},"anthropic":{"active":"work","accounts":{"default":{"type":"api_key","access":"a"},"work":{"type":"api_key","access":"b"}}}}"#).unwrap();
        assert_eq!(
            secrets[&(String::from("openai"), String::from("default"))].value,
            "key"
        );
        assert_eq!(
            secrets[&(String::from("anthropic"), String::from("work"))].value,
            "b"
        );
        assert_eq!(active["anthropic"], "work");
    }
}
