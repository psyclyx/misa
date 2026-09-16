//! Choosing from a set of candidates.
//!
//! # The boundary
//!
//! Here: filtering, ranking, frecency, the selected index, and what accepting
//! produces. None of it needs a session, a network, or a round trip, and all of it
//! differs between a terminal, a browser, and a remote control.
//!
//! Not here: *which* candidates exist. A session declares where a value can come
//! from ([`crate::intent::Source`]) and provides the items — a resident source as
//! a query a client holds, an on-demand source as an answer to
//! [`crate::intent::Intent::Complete`].
//!
//! # Why the split pays
//!
//! It moves a decision from "on every keystroke" to "once, ahead of time". A client
//! with the declaration and the items opens a picker, filters, and settles an
//! argument with nothing in flight. A session that owned the picker would see every
//! keystroke of a search, rank lists it knows nothing about, and still have to leave
//! the appearance to the platform — which is why the previous system's choice module
//! was client-side, and why the *declaration* is the thing that makes that safe.

use misa_proto::view::Choice;
use crate::intent::Intent;
use misa_value::Value;

/// What accepting a candidate does.
#[derive(Clone, Debug, PartialEq)]
pub enum Accept {
    /// Fill one argument of a declared command. Accepting does *not* send it: an
    /// argument is part of a line somebody is still writing.
    Argument { command: String, argument: String },
    /// Run the command itself.
    Run,
    /// Resolve an action the view offered.
    Action { node: String, action: String },
}

/// A candidate a person settled on, and what to do about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Accepted {
    pub value: String,
    pub label: String,
    pub accept: Accept,
}

/// What a keystroke in a picker caused.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Nothing beyond the picker's own state.
    None,
    /// The query moved and this source has to be asked. Only ever produced by a
    /// picker that holds nothing for that source.
    Ask { source: String, prefix: String },
    /// Somebody settled on a candidate.
    Accepted(Accepted),
    /// Somebody dismissed the picker.
    Cancelled,
}

/// A picker's state.
pub struct Picker {
    pub title: String,
    pub accept: Accept,
    /// Which source the items came from, when they came from one.
    pub source: Option<String>,
    items: Vec<Choice>,
    /// Whether the items are everything the source has.
    pub truncated: bool,
    pub query: String,
    selected: usize,
    frecency: Frecency,
}

impl Picker {
    pub fn new(title: impl Into<String>, accept: Accept) -> Picker {
        Picker {
            title: title.into(),
            accept,
            source: None,
            items: Vec::new(),
            truncated: false,
            query: String::new(),
            selected: 0,
            frecency: Frecency::default(),
        }
    }

    /// A picker over a source that may need asking for candidates as it filters.
    pub fn over(source: impl Into<String>, title: impl Into<String>, accept: Accept) -> Picker {
        let mut picker = Picker::new(title, accept);
        picker.source = Some(source.into());
        picker
    }

    pub fn with_frecency(mut self, frecency: Frecency) -> Picker {
        self.frecency = frecency;
        self
    }

    /// Replace the candidate set: a resident source's items, or an on-demand answer.
    pub fn set_items(&mut self, items: Vec<Choice>, truncated: bool) {
        self.items = items;
        self.truncated = truncated;
        self.selected = 0;
    }

    pub fn items(&self) -> &[Choice] {
        &self.items
    }

    /// Whether what is held is all there is. A client that cannot say this would
    /// pretend a prefix had no match.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn frecency(&self) -> &Frecency {
        &self.frecency
    }

    /// The candidates whose match survives the query, best first.
    ///
    /// Ranked rather than merely filtered: somebody typing `scr` at a model list
    /// wants `scripted-1` and not `scripted-chatty` first, and that difference is the
    /// whole of what makes a picker pleasant. Frecency breaks ties, so what a person
    /// chooses often comes up first with no query at all.
    pub fn matches(&self) -> Vec<&Choice> {
        let needle = self.query.trim();
        if needle.is_empty() {
            let mut all: Vec<&Choice> = self.items.iter().collect();
            all.sort_by(|left, right| self.frecency.score(&right.value).cmp(&self.frecency.score(&left.value)));
            return all;
        }
        let mut scored: Vec<(i64, usize, &Choice)> = Vec::new();
        for (index, candidate) in self.items.iter().enumerate() {
            let haystack = format!("{} {}", candidate.value, candidate.label);
            if let Some(score) = match_score(needle, &haystack) {
                // A better match always wins; frecency orders equal matches, and the
                // original order breaks the last tie so the list is stable.
                let frecency = self.frecency.score(&candidate.value).signum();
                scored.push((score * 1000 + frecency, index, candidate));
            }
        }
        scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        scored.into_iter().map(|(_, _, candidate)| candidate).collect()
    }

    /// The candidate somebody would accept right now.
    pub fn selected(&self) -> Option<&Choice> {
        let matches = self.matches();
        matches.get(self.selected.min(matches.len().saturating_sub(1))).copied()
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Move the selection, wrapping. Deliberate: a picker with eight items should
    /// not need a third key to reach the other end.
    pub fn move_selection(&mut self, delta: isize) {
        let count = self.matches().len();
        if count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(count as isize) as usize;
    }

    pub fn type_char(&mut self, character: char) -> Effect {
        self.query.push(character);
        self.selected = 0;
        self.ask()
    }

    pub fn backspace(&mut self) -> Effect {
        self.query.pop();
        self.selected = 0;
        self.ask()
    }

    pub fn set_query(&mut self, query: impl Into<String>) -> Effect {
        self.query = query.into();
        self.selected = 0;
        self.ask()
    }

    /// Ask for candidates only when the source cannot answer from what is held.
    fn ask(&self) -> Effect {
        let Some(source) = &self.source else {
            return Effect::None;
        };
        if self.items.is_empty() {
            return Effect::Ask { source: source.clone(), prefix: self.query.clone() };
        }
        Effect::None
    }

    /// Settle on the selected candidate.
    pub fn accept(&mut self) -> Effect {
        let Some(candidate) = self.selected().cloned() else {
            return Effect::None;
        };
        self.frecency.bump(&candidate.value);
        Effect::Accepted(Accepted { value: candidate.value, label: candidate.label, accept: self.accept.clone() })
    }

    pub fn cancel(&self) -> Effect {
        Effect::Cancelled
    }

    /// The line an accepted argument completes.
    pub fn fill_text(accepted: &Accepted) -> String {
        match &accepted.accept {
            Accept::Argument { command, .. } => format!("/{} {}", command, crate::intent::quote(&accepted.value)),
            Accept::Run => accepted.value.clone(),
            Accept::Action { .. } => accepted.value.clone(),
        }
    }

    /// The intent an accepted candidate becomes, when it is not filling a line.
    pub fn intent(accepted: &Accepted, fields: Vec<misa_proto::view::Field>) -> Option<Intent> {
        match &accepted.accept {
            Accept::Argument { .. } => None,
            Accept::Run => Some(Intent::Command {
                name: accepted.value.trim_start_matches('/').to_string(),
                args: Value::Null,
            }),
            Accept::Action { node, action } => Some(Intent::Action {
                node: node.clone(),
                action: action.clone(),
                args: Value::str(&accepted.value),
                fields,
            }),
        }
    }
}

/// How well a needle matches a haystack, or `None` when it does not.
///
/// A subsequence match with a bonus at a word boundary and a bonus for consecutive
/// characters. That is the whole rule, and it is on purpose: a fancier one needs a
/// corpus to tune against, and the previous system grew a great deal of machinery
/// around a rule that fits in one function.
pub fn match_score(needle: &str, haystack: &str) -> Option<i64> {
    if needle.is_empty() {
        return Some(0);
    }
    let needle: Vec<char> = needle.to_lowercase().chars().collect();
    let haystack: Vec<char> = haystack.to_lowercase().chars().collect();
    let mut score = 0i64;
    let mut at = 0usize;
    let mut last: Option<usize> = None;
    for (index, character) in haystack.iter().enumerate() {
        if at >= needle.len() {
            break;
        }
        if *character != needle[at] {
            continue;
        }
        score += 10;
        // A run of matches is what somebody means by typing the start of a word, and
        // it has to outweigh the length penalty below or a scattered match in a long
        // name would beat a prefix.
        if last == Some(index.wrapping_sub(1)) {
            score += 25;
        }
        if index == 0 || matches!(haystack.get(index.wrapping_sub(1)), Some('-') | Some('_') | Some('/') | Some(' ')) {
            score += 20;
        }
        last = Some(index);
        at += 1;
    }
    if at < needle.len() {
        return None;
    }
    // A shorter haystack that matched the same needle is the more specific answer,
    // but mildly: specificity breaks ties and never beats a better match.
    Some(score - haystack.len() as i64 / 4)
}

/// What somebody chooses often, which is worth remembering.
///
/// Presentation state, and it belongs to a client: two terminals on one session may
/// have very different habits, and a session that kept this would be keeping a
/// record of somebody's typing for no reason it could justify.
#[derive(Default, Clone, Debug)]
pub struct Frecency {
    counts: std::collections::BTreeMap<String, i64>,
}

impl Frecency {
    pub fn bump(&mut self, value: &str) {
        *self.counts.entry(value.to_string()).or_insert(0) += 1;
    }

    pub fn score(&self, value: &str) -> i64 {
        self.counts.get(value).copied().unwrap_or(0)
    }

    pub fn counts(&self) -> &std::collections::BTreeMap<String, i64> {
        &self.counts
    }

    pub fn from_counts(counts: std::collections::BTreeMap<String, i64>) -> Frecency {
        Frecency { counts }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_choices() -> Vec<Choice> {
        vec![
            Choice { value: "scripted-1".into(), label: "Scripted".into(), detail: None },
            Choice { value: "scripted-chatty".into(), label: "Scripted chatty".into(), detail: None },
            Choice { value: "gpt-5".into(), label: "GPT 5".into(), detail: None },
            Choice { value: "claude-sonnet-5".into(), label: "Claude Sonnet 5".into(), detail: None },
        ]
    }

    fn picker() -> Picker {
        let mut picker = Picker::over(
            "models",
            "Model",
            Accept::Argument { command: "model".into(), argument: "model".into() },
        );
        picker.set_items(model_choices(), false);
        picker
    }

    #[test]
    fn a_subsequence_matches_and_a_missing_character_does_not() {
        assert!(match_score("scr", "scripted-1").is_some());
        assert!(match_score("g5", "GPT 5").is_some());
        assert!(match_score("zzz", "scripted-1").is_none());
        assert!(match_score("", "anything").is_some());
    }

    #[test]
    fn a_word_start_and_a_run_of_matches_both_raise_the_score() {
        let boundary = match_score("s", "s-c-r").expect("a boundary match");
        let middle = match_score("c", "scripted-c").expect("a middle match");
        assert!(boundary > middle, "a word start should be worth more than a middle match");
        let run = match_score("scr", "scripted").expect("a run");
        let scattered = match_score("scr", "s-c-r-ipted").expect("scattered");
        assert!(run > scattered, "a run should beat scattered matches");
    }

    #[test]
    fn a_query_filters_and_the_best_match_is_first() {
        let mut picker = picker();
        picker.set_query("scripted");
        let matches = picker.matches();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].value, "scripted-1", "the shorter match should lead");
        assert_eq!(picker.selected().expect("a selection").value, "scripted-1");
    }

    #[test]
    fn a_query_matching_nothing_leaves_nothing_to_accept() {
        let mut picker = picker();
        picker.set_query("zzzz");
        assert!(picker.matches().is_empty());
        assert_eq!(picker.accept(), Effect::None);
    }

    #[test]
    fn selection_wraps_in_both_directions() {
        let mut picker = picker();
        picker.move_selection(-1);
        assert_eq!(picker.selected_index(), 3, "moving up from the first wraps to the last");
        picker.move_selection(1);
        assert_eq!(picker.selected_index(), 0);
    }

    #[test]
    fn frecency_orders_an_empty_query_and_never_overrides_a_better_match() {
        let mut frecency = Frecency::default();
        frecency.bump("gpt-5");
        frecency.bump("gpt-5");
        let mut picker = picker().with_frecency(frecency);
        assert_eq!(picker.matches()[0].value, "gpt-5");
        picker.set_query("claude");
        assert_eq!(picker.matches()[0].value, "claude-sonnet-5");
    }

    #[test]
    fn accepting_remembers_what_was_chosen() {
        let mut picker = picker();
        picker.set_query("gpt");
        let Effect::Accepted(accepted) = picker.accept() else {
            panic!("expected an acceptance");
        };
        assert_eq!(accepted.value, "gpt-5");
        assert_eq!(picker.frecency().score("gpt-5"), 1);
    }

    #[test]
    fn a_picker_over_a_resident_source_never_asks_the_session() {
        let mut picker = picker();
        assert_eq!(picker.type_char('s'), Effect::None);
        assert_eq!(picker.type_char('c'), Effect::None);
        assert_eq!(picker.query, "sc");
    }

    #[test]
    fn a_picker_that_holds_nothing_asks_for_a_prefix() {
        let mut picker = Picker::over(
            "conversations",
            "Conversation",
            Accept::Argument { command: "resume".into(), argument: "conversation".into() },
        );
        assert_eq!(
            picker.type_char('p'),
            Effect::Ask { source: "conversations".into(), prefix: "p".into() }
        );
        picker.set_items(vec![Choice { value: "c2".into(), label: "parser".into(), detail: None }], true);
        assert!(picker.is_truncated());
        assert_eq!(picker.type_char('a'), Effect::None, "a held source was asked anyway");
    }

    #[test]
    fn an_accepted_argument_fills_a_line_and_does_not_send_it() {
        let accepted = Accepted {
            value: "claude-sonnet-5".into(),
            label: "Claude Sonnet 5".into(),
            accept: Accept::Argument { command: "model".into(), argument: "model".into() },
        };
        assert_eq!(Picker::fill_text(&accepted), "/model claude-sonnet-5");
        assert!(Picker::intent(&accepted, Vec::new()).is_none(), "accepting an argument sent the command");
    }

    #[test]
    fn accepting_a_command_runs_it_and_the_slash_is_not_sent() {
        let accepted = Accepted { value: "/clear".into(), label: "/clear".into(), accept: Accept::Run };
        assert_eq!(Picker::fill_text(&accepted), "/clear");
        match Picker::intent(&accepted, Vec::new()).expect("an intent") {
            Intent::Command { name, .. } => assert_eq!(name, "clear"),
            other => panic!("expected a command, got {other:?}"),
        }
    }

    #[test]
    fn a_query_change_on_an_empty_source_is_the_only_thing_that_reaches_a_session() {
        let mut picker = Picker::over("conversations", "Conversation", Accept::Run);
        assert!(matches!(picker.set_query("a"), Effect::Ask { .. }));
    }
}
