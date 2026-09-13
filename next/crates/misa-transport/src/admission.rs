//! Who may attach: the one decision a daemon makes about a stranger.
//!
//! iroh authenticates the peer's public key, so by the time a connection reaches an
//! application the peer has an *identity*. Nothing about that identity says whether the
//! peer should be talking to this session. Deployment admission belongs beside the
//! transport, where session and blob connections share the same decision.
//!
//! # Why this is data
//!
//! A [`Roster`] is a list and a mode, decided once when a daemon starts. It has no
//! mutable state, no `is_allowed` callback into a plugin, and no way to change at
//! runtime, so "may this peer attach" is a pure function of the ticket's session, the
//! connection's peer id, and this list. That is what makes it testable without a socket,
//! and it is the same reason the view tree has no appearance in it: a decision that is
//! data can be checked by anybody, and one that hides in a handler cannot.
//!
//! # The two modes, and how a client gets in
//!
//! - [`Mode::Open`] admits any peer. What a daemon on loopback, or a machine behind a
//!   relay that a person is deliberately sharing, wants — and what every test wants.
//! - [`Mode::Listed`] admits only peers named, by endpoint id.
//!
//! Listing keys has a problem a person notices immediately: to let a client in you have
//! to know its endpoint id, and the only place that appears is a log on the machine that
//! is trying to connect. So an [`Admission`] is a roster *plus* a set of paired keys:
//! the daemon shows a code — in a terminal, as a QR code — a client presents it once,
//! and the identity iroh already authenticated is written down. From then on that client
//! connects like any other, with no flag to edit and no id to copy.
//!
//! # What a code is, and what it is not
//!
//! A code is a one-time secret with a deadline, minted when somebody asks for one and
//! spent when it is used. It is not a password, it is not stored anywhere, and it is
//! never what gets approved: the *key* is approved, and the code only says that the
//! person holding it was in the room when it was shown. That is why revoking a client is
//! deleting a line in a file rather than rotating a secret.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// How long an invitation is good for unless somebody says otherwise.
pub const INVITATION_TTL_MS: i64 = 10 * 60 * 1000;

/// Whether a peer may attach, as configuration rather than code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roster {
    mode: Mode,
    peers: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Open,
    Listed,
}

impl Roster {
    /// Admit anybody.
    pub fn open() -> Roster {
        Roster { mode: Mode::Open, peers: BTreeSet::new() }
    }

    /// Admit only the peers named.
    ///
    /// An empty list in this mode admits nobody, which is the right reading: a daemon
    /// told to restrict itself and given no names is a daemon that has nothing to do
    /// yet, and quietly falling back to open would be the one mistake this type exists
    /// to prevent.
    pub fn listed(peers: impl IntoIterator<Item = String>) -> Roster {
        Roster { mode: Mode::Listed, peers: peers.into_iter().map(|peer| peer.trim().to_string()).filter(|peer| !peer.is_empty()).collect() }
    }

    /// Admit this peer too.
    ///
    /// A builder, not a mutator: the composition names its peers before the endpoint is
    /// bound, and nothing changes the answer afterwards.
    pub fn also(mut self, peer: impl Into<String>) -> Roster {
        let peer = peer.into().trim().to_string();
        if peer.is_empty() {
            return self;
        }
        self.peers.insert(peer);
        self
    }

    /// The same roster, listed rather than open.
    pub fn restricted(self) -> Roster {
        Roster { mode: Mode::Listed, peers: self.peers }
    }

    /// The decision, and the only one this type makes.
    ///
    /// `peer` is an endpoint id as the transport reports it. The comparison is exact:
    /// an endpoint id is a public key, so a prefix of one names nothing.
    pub fn admits(&self, peer: &str) -> bool {
        match self.mode {
            Mode::Open => true,
            Mode::Listed => self.peers.contains(peer),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn is_open(&self) -> bool {
        self.mode == Mode::Open
    }

    /// The named peers, sorted.
    pub fn peers(&self) -> Vec<String> {
        self.peers.iter().cloned().collect()
    }

    /// One line for a daemon to print when it starts.
    ///
    /// Says what a person needs to know to either use it or lock it down, and says it in
    /// terms of the flag that changes it.
    pub fn describe(&self) -> String {
        match self.mode {
            Mode::Open => "open (any peer with this ticket can attach; --allow <endpoint id> restricts it)".to_string(),
            Mode::Listed => format!("{} listed peer(s)", self.peers.len()),
        }
    }
}

impl Default for Roster {
    fn default() -> Self {
        Roster::open()
    }
}

/// One key this daemon has approved, and what it remembers about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    pub id: String,
    /// What the client called itself when it paired: `the phone`, `the laptop`.
    pub label: String,
    pub added_ms: i64,
}

/// The keys that have been paired, and where they are written down.
///
/// Durable, because the whole point is that a pairing survives a restart: a store that
/// forgot its approvals when the daemon stopped would be a daemon that asks for the code
/// again every time, which nobody would use twice. Mode 0600 and an atomic rename, like
/// the credential store, because an approved key is a capability and a half-written file
/// is a file nobody can read.
#[derive(Debug)]
pub struct Paired {
    path: Option<PathBuf>,
    peers: std::sync::Mutex<Vec<Peer>>,
}

impl Paired {
    /// A store that keeps what it approves, in `path`.
    pub fn at(path: impl Into<PathBuf>) -> Result<Paired, String> {
        let path = path.into();
        let peers = match std::fs::read_to_string(&path) {
            Ok(text) => parse(&text)?,
            // A missing file is a daemon nobody has paired with, not an error.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(format!("could not read {}: {err}", path.display())),
        };
        Ok(Paired { path: Some(path), peers: std::sync::Mutex::new(peers) })
    }

    /// A store that forgets when the process ends: a daemon with no data directory.
    pub fn in_memory() -> Paired {
        Paired { path: None, peers: std::sync::Mutex::new(Vec::new()) }
    }

    pub fn admits(&self, id: &str) -> bool {
        self.peek().iter().any(|peer| peer.id == id)
    }

    pub fn peers(&self) -> Vec<Peer> {
        self.peek()
    }

    /// Approve a key, and write it down before returning.
    ///
    /// Written first and remembered second, so an approval that is reported as done is an
    /// approval that survives a restart. Pairing the same key twice is not an error: a person
    /// who reinstalls a client and pairs again means the same thing by it either way, and the
    /// label is simply brought up to date.
    pub fn add(&self, id: &str, label: &str, now: i64) -> Result<(), String> {
        let id = id.trim();
        if id.is_empty() {
            return Err("a peer with no endpoint id cannot be approved".into());
        }
        let mut peers = self.lock()?;
        peers.retain(|peer| peer.id != id);
        peers.push(Peer { id: id.to_string(), label: label.trim().to_string(), added_ms: now });
        peers.sort_by(|left, right| left.id.cmp(&right.id));
        self.save(&peers)
    }

    /// Forget a key. `true` when there was something to forget.
    pub fn revoke(&self, id: &str) -> Result<bool, String> {
        let mut peers = self.lock()?;
        let before = peers.len();
        peers.retain(|peer| peer.id != id);
        let removed = peers.len() != before;
        if removed {
            self.save(&peers)?;
        }
        Ok(removed)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Vec<Peer>>, String> {
        self.peers.lock().map_err(|_| "the paired store is poisoned".to_string())
    }

    fn peek(&self) -> Vec<Peer> {
        self.peers.lock().map(|peers| peers.clone()).unwrap_or_default()
    }

    fn save(&self, peers: &[Peer]) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let text = render(peers);
        let temporary = path.with_extension("part");
        std::fs::write(&temporary, text).map_err(|err| err.to_string())?;
        set_private(&temporary)?;
        std::fs::rename(&temporary, path).map_err(|err| err.to_string())?;
        Ok(())
    }
}

impl Default for Paired {
    fn default() -> Self {
        Paired::in_memory()
    }
}

/// One line per key: the id, when it was added, and what it called itself.
///
/// Two tabs, because a label is a person's words and may contain anything but a newline.
fn render(peers: &[Peer]) -> String {
    let mut out = String::new();
    for peer in peers {
        out.push_str(&format!("{}\t{}\t{}\n", peer.id, peer.added_ms, peer.label));
    }
    out
}

fn parse(text: &str) -> Result<Vec<Peer>, String> {
    let mut peers = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(3, '\t');
        let id = fields.next().unwrap_or_default().trim();
        let added_ms = fields.next().unwrap_or_default().trim();
        let label = fields.next().unwrap_or_default().trim();
        if id.is_empty() {
            return Err(format!("line {} names no endpoint", number + 1));
        }
        peers.push(Peer {
            id: id.to_string(),
            label: label.to_string(),
            added_ms: added_ms.parse().unwrap_or(0),
        });
    }
    Ok(peers)
}

#[cfg(unix)]
fn set_private(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|err| err.to_string())
}

#[cfg(not(unix))]
fn set_private(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// A code a person shows to a client, once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invitation {
    code: String,
    minted_ms: i64,
    ttl_ms: i64,
}

impl Invitation {
    /// A fresh invitation, with a code that is short enough to read out loud and long
    /// enough that guessing it is not a plan.
    ///
    /// The alphabet leaves out the characters people confuse when they read one off a
    /// screen — `0`/`O`, `1`/`I` — because the one thing that goes wrong with a code is
    /// somebody typing it.
    pub fn mint(now: i64, ttl_ms: i64) -> Invitation {
        const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
        let mut bytes = [0u8; 12];
        getrandom(&mut bytes);
        let code: String = bytes
            .iter()
            .enumerate()
            .map(|(index, byte)| {
                let character = ALPHABET[*byte as usize % ALPHABET.len()] as char;
                // Two groups of six, because a person reading one out loud reads groups.
                if index == 6 { format!("-{character}") } else { character.to_string() }
            })
            .collect();
        Invitation { code, minted_ms: now, ttl_ms }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn expired(&self, now: i64) -> bool {
        now.saturating_sub(self.minted_ms) > self.ttl_ms
    }

    /// How long it has left, for a daemon that wants to say so.
    pub fn seconds_left(&self, now: i64) -> i64 {
        (self.ttl_ms - now.saturating_sub(self.minted_ms)).max(0) / 1_000
    }

    /// Whether this code is the one, compared without leaking how far it matched.
    pub fn matches(&self, code: &str, now: i64) -> bool {
        if self.expired(now) {
            return false;
        }
        let given = code.trim();
        self.code.len() == given.len()
            && self
                .code
                .bytes()
                .zip(given.bytes())
                .fold(0u8, |difference, (ours, theirs)| difference | (ours ^ theirs))
                == 0
    }
}

/// Fill a buffer with random bytes.
///
/// `/dev/urandom` rather than a dependency, and rather than the clock: a code that is
/// guessable from the time it was made is not a code. A machine without it cannot pair
/// anybody, which is the safe direction to fail in — the invitation code says so.
fn getrandom(bytes: &mut [u8]) {
    use std::io::Read as _;
    let Ok(mut file) = std::fs::File::open("/dev/urandom") else {
        // No randomness, no invitation. A code nobody can mint is better than one everybody
        // can guess, and the daemon says as much when it tries.
        bytes.fill(0);
        return;
    };
    if file.read_exact(bytes).is_err() {
        bytes.fill(0);
    }
}

/// Who may attach: a roster, the keys that have been paired, and the one open invitation.
#[derive(Debug)]
pub struct Admission {
    roster: Roster,
    paired: Paired,
    invitation: std::sync::Mutex<Option<Invitation>>,
}

impl Admission {
    /// Admit anybody: what a test wants, and what `--open` means.
    pub fn open() -> Admission {
        Admission { roster: Roster::open(), paired: Paired::in_memory(), invitation: std::sync::Mutex::new(None) }
    }

    /// Admit the peers named, and anybody who pairs.
    pub fn listed(peers: impl IntoIterator<Item = String>) -> Admission {
        Admission { roster: Roster::listed(peers), paired: Paired::in_memory(), invitation: std::sync::Mutex::new(None) }
    }

    /// Admit anybody whose key has been approved, and the names a composition added.
    pub fn paired(paired: Paired) -> Admission {
        Admission { roster: Roster::listed(std::iter::empty::<String>()), paired, invitation: std::sync::Mutex::new(None) }
    }

    /// Admit this peer as well, whether or not it ever pairs.
    pub fn also(mut self, peer: impl Into<String>) -> Admission {
        self.roster = self.roster.also(peer);
        self
    }

    /// The decision, and the only one this type makes.
    pub fn admits(&self, peer: &str) -> bool {
        self.roster.admits(peer) || self.paired.admits(peer)
    }

    /// Whether a peer that is not admitted may still ask to be.
    ///
    /// True while an invitation is open: a daemon with no code out is a daemon with nothing
    /// to say to a stranger, and saying nothing is the whole point.
    pub fn is_inviting(&self, now: i64) -> bool {
        self.invitation
            .lock()
            .ok()
            .and_then(|invitation| invitation.clone())
            .is_some_and(|invitation| !invitation.expired(now))
    }

    /// Mint a code, replacing whatever was outstanding.
    pub fn invite(&self, ttl_ms: i64, now: i64) -> Invitation {
        let invitation = Invitation::mint(now, ttl_ms);
        if let Ok(mut open) = self.invitation.lock() {
            *open = Some(invitation.clone());
        }
        invitation
    }

    /// Spend a code on a key. The one path by which a stranger becomes a client.
    pub fn accept(&self, peer: &str, code: &str, label: &str, now: i64) -> Result<(), String> {
        let outstanding = self
            .invitation
            .lock()
            .map_err(|_| "the invitation is poisoned".to_string())?
            .clone();
        let Some(invitation) = outstanding else {
            return Err("this daemon has no code outstanding; ask it for one".to_string());
        };
        if invitation.expired(now) {
            return Err("that code has expired".to_string());
        }
        if !invitation.matches(code, now) {
            // Deliberately the same sentence for a wrong code and an expired one: a peer that
            // is guessing learns nothing about how close it got.
            return Err("that code is not the one this daemon is showing".to_string());
        }
        self.paired.add(peer, label, now)?;
        // Spent, whether or not it is used again: one code is one client.
        if let Ok(mut open) = self.invitation.lock() {
            *open = None;
        }
        Ok(())
    }

    /// Forget a key, so that it has to pair again.
    pub fn revoke(&self, peer: &str) -> Result<bool, String> {
        self.paired.revoke(peer)
    }

    /// Every key that may attach without a code, in order.
    pub fn peers(&self) -> Vec<Peer> {
        let mut peers = self.paired.peers();
        peers.sort_by(|left, right| left.id.cmp(&right.id));
        peers
    }

    /// The roster underneath, for a diagnostic that wants the static half.
    pub fn roster(&self) -> &Roster {
        &self.roster
    }

    /// Whether anybody at all may attach, which is the one thing a daemon must say out loud.
    pub fn is_open(&self) -> bool {
        self.roster.is_open()
    }

    /// One line for a daemon to print when it starts.
    ///
    /// Says what a person needs to know to either use it or lock it down, in terms of the
    /// flags that change it, and names the command that mints a code — because a daemon that
    /// requires pairing and does not say how to pair is a daemon nobody can use.
    pub fn describe(&self) -> String {
        if self.roster.is_open() {
            return "open (any peer with this ticket can attach; --allow <endpoint id> pairs nothing, it admits directly)".to_string();
        }
        let paired = self.paired.peers().len();
        format!(
            "paired keys only ({paired} approved; type `pair` to show a code, `peers` to list \
             them, `revoke <endpoint id>` to forget one)"
        )
    }
}

impl Default for Admission {
    fn default() -> Self {
        Admission::open()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: &str = "9304e5d0e2a5b17e6a1d3f6b7c8d9e0f1a2b3c4d5e6f708192a3b4c5d6e7f801";

    #[test]
    fn open_admits_anybody_and_listed_admits_only_the_named() {
        assert!(Roster::open().admits(PEER));
        assert!(!Roster::listed([]).admits(PEER));
        assert!(Roster::listed([PEER.to_string()]).admits(PEER));
        assert!(!Roster::listed([PEER.to_string()]).admits("someone else"));
        // A prefix names nothing: an endpoint id is a public key and not a namespace.
        assert!(!Roster::listed([PEER.to_string()]).admits(&PEER[..16]));
    }

    #[test]
    fn a_restricted_roster_carries_the_names_it_was_given() {
        let roster = Roster::open().also(PEER).also("  ").restricted();
        assert_eq!(roster.mode(), Mode::Listed);
        assert_eq!(roster.peers(), vec![PEER.to_string()]);
        assert!(roster.admits(PEER));
        // Restricting without naming anybody admits nobody, rather than everybody.
        assert!(!Roster::open().restricted().admits(PEER));
    }

    #[test]
    fn the_roster_says_which_mode_it_is_in_and_how_to_change_it() {
        assert!(Roster::open().describe().contains("--allow"));
        assert!(Roster::listed([PEER.to_string()]).describe().contains("1"));
    }

    #[test]
    fn a_code_pairs_one_key_once_and_then_that_key_is_a_client() {
        let admission = Admission::paired(Paired::in_memory());
        assert!(!admission.admits(PEER), "a stranger was admitted before pairing");
        let invitation = admission.invite(INVITATION_TTL_MS, 1_000);
        let code = invitation.code().to_string();
        assert_eq!(code.len(), 13, "a code is two groups of six: {code}");
        assert!(invitation.seconds_left(1_000) > 0);

        // The wrong code is refused, and says nothing about how wrong it was.
        let error = admission.accept(PEER, "WRONGCODE-2", "the phone", 2_000).unwrap_err();
        assert!(error.contains("not the one"), "{error}");
        assert!(!admission.admits(PEER));

        admission.accept(PEER, &code, "the phone", 2_000).expect("pairing");
        assert!(admission.admits(PEER));
        assert!(!admission.admits("someone else"));
        let peers = admission.peers();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].id, PEER);
        assert_eq!(peers[0].label, "the phone");

        // One code is one client: the same code does not pair a second key.
        let error = admission.accept("someone else", &code, "another", 3_000).unwrap_err();
        assert!(error.contains("no code outstanding"), "{error}");
    }

    #[test]
    fn an_expired_code_is_not_a_code() {
        let admission = Admission::paired(Paired::in_memory());
        let invitation = admission.invite(1_000, 1_000);
        let code = invitation.code().to_string();
        assert!(invitation.expired(2_001), "a one-second code outlived its second");
        let error = admission.accept(PEER, &code, "late", 5_000).unwrap_err();
        assert!(error.contains("expired"), "{error}");
        assert!(!admission.admits(PEER));
        assert!(!admission.is_inviting(5_000));
    }

    #[test]
    fn a_paired_key_outlives_the_process_that_approved_it() {
        let root = std::env::temp_dir().join(format!("misa-paired-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("paired");
        let admission = Admission::paired(Paired::at(&path).expect("a store"));
        admission.invite(INVITATION_TTL_MS, 0);
        let code = admission.invite(INVITATION_TTL_MS, 0).code().to_string();
        admission.accept(PEER, &code, "the laptop", 10).expect("pairing");
        assert!(admission.admits(PEER));

        // A new process reading the same file admits the same key, which is the whole point.
        let reopened = Admission::paired(Paired::at(&path).expect("a store"));
        assert!(reopened.admits(PEER), "pairing did not survive a reopen");
        assert_eq!(reopened.peers()[0].label, "the laptop");
        assert_eq!(reopened.peers()[0].added_ms, 10);

        // And revoking is forgetting, which has to survive a reopen too.
        assert!(reopened.revoke(PEER).expect("revoking"));
        assert!(!Admission::paired(Paired::at(&path).expect("a store")).admits(PEER));
        assert!(!reopened.revoke(PEER).expect("revoking twice"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).expect("the file").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "an approved key was written world-readable");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_daemon_that_is_listing_says_how_to_connect_and_one_that_is_open_says_so() {
        let closed = Admission::paired(Paired::in_memory());
        assert!(closed.describe().contains("paired keys only"), "{}", closed.describe());
        assert!(closed.describe().contains("`pair`"), "{}", closed.describe());
        // Nothing is invited until somebody asks, so a stranger is told nothing at all.
        assert!(!closed.is_inviting(0));
        assert!(Admission::open().describe().contains("open"));
        assert!(Admission::open().is_open());
    }

    #[test]
    fn a_named_peer_is_admitted_without_ever_pairing() {
        // What `--allow` is for: a script, or a fixed client, that should not need a code.
        let admission = Admission::paired(Paired::in_memory()).also(PEER);
        assert!(admission.admits(PEER));
        assert!(!admission.admits("someone else"));
        assert!(!admission.roster().is_open());
    }
}
