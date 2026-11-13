//! Spell checking, offline, with the American English dictionary built in (see `dict/`).
//!
//! The dictionary is in Hunspell's format: word stems with flags, and an affix file whose
//! rules make the other forms (plurals, past tenses and so on). The English affix rules are
//! plain prefixes and suffixes, so every form is made once, when the dictionary is first
//! used, and kept in a set. Suggestions are the known words one or two edits away.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::OnceLock;

const AFF: &str = include_str!("../dict/en_US.aff");
const DIC: &str = include_str!("../dict/en_US.dic");

/// Longest word looked at; anything longer isn't a word.
const MAX_WORD: usize = 64;

/// One element of an affix condition.
#[derive(Debug, Clone)]
enum Cond {
    Any,
    In(Vec<char>),
    NotIn(Vec<char>),
}

impl Cond {
    fn matches(&self, c: char) -> bool {
        match self {
            Cond::Any => true,
            Cond::In(set) => set.contains(&c),
            Cond::NotIn(set) => !set.contains(&c),
        }
    }
}

fn parse_condition(s: &str) -> Vec<Cond> {
    let mut out = Vec::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '.' => out.push(Cond::Any),
            '[' => {
                let mut set = Vec::new();
                let mut negated = false;
                let mut first = true;
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    if first && c == '^' {
                        negated = true;
                    } else {
                        set.push(c);
                    }
                    first = false;
                }
                out.push(if negated { Cond::NotIn(set) } else { Cond::In(set) });
            }
            c => out.push(Cond::In(vec![c])),
        }
    }
    out
}

#[derive(Debug, Clone)]
struct Rule {
    strip: String,
    add: String,
    condition: Vec<Cond>,
}

#[derive(Debug, Clone, Default)]
struct Affix {
    prefix: bool,
    cross: bool,
    rules: Vec<Rule>,
}

impl Rule {
    /// The word with this rule applied, when its condition holds.
    fn apply(&self, word: &str, prefix: bool) -> Option<String> {
        let chars: Vec<char> = word.chars().collect();
        let n = self.condition.len();
        if chars.len() < n {
            return None;
        }
        let part = if prefix {
            chars.get(..n)?
        } else {
            chars.get(chars.len() - n..)?
        };
        if !part.iter().zip(&self.condition).all(|(c, cond)| cond.matches(*c)) {
            return None;
        }
        if prefix {
            let rest = word.strip_prefix(self.strip.as_str())?;
            Some(format!("{}{rest}", self.add))
        } else {
            let rest = word.strip_suffix(self.strip.as_str())?;
            Some(format!("{rest}{}", self.add))
        }
    }
}

/// A spelling dictionary: every form of every word.
pub struct Dictionary {
    words: HashSet<String>,
    /// Letters to try in suggestions, most common first.
    try_chars: Vec<char>,
}

fn parse_affixes(aff: &str) -> (HashMap<char, Affix>, Vec<char>, HashSet<char>) {
    let mut affixes: HashMap<char, Affix> = HashMap::new();
    let mut try_chars = Vec::new();
    let mut skip = HashSet::new();
    for line in aff.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            ["TRY", letters, ..] => try_chars = letters.chars().collect(),
            // Flags for words that only appear inside compounds (1th, 2th): not words alone.
            ["ONLYINCOMPOUND", flag, ..] | ["NEEDAFFIX", flag, ..] => skip.extend(flag.chars().next()),
            [kind @ ("PFX" | "SFX"), flag, cross, count] if count.parse::<usize>().is_ok() => {
                let Some(flag) = flag.chars().next() else { continue };
                let affix = affixes.entry(flag).or_default();
                affix.prefix = *kind == "PFX";
                affix.cross = *cross == "Y";
            }
            ["PFX" | "SFX", flag, strip, add, condition, ..] => {
                let Some(flag) = flag.chars().next() else { continue };
                let zero = |s: &str| if s == "0" { String::new() } else { s.to_owned() };
                // Continuation flags on the added part aren't used by this dictionary.
                let add = add.split('/').next().unwrap_or_default();
                affixes.entry(flag).or_default().rules.push(Rule {
                    strip: zero(strip),
                    add: zero(add),
                    condition: parse_condition(condition),
                });
            }
            _ => {}
        }
    }
    (affixes, try_chars, skip)
}

impl Dictionary {
    /// Reads a Hunspell dictionary whose affixes are plain prefixes and suffixes.
    pub fn parse(aff: &str, dic: &str) -> Self {
        let (affixes, try_chars, skip) = parse_affixes(aff);
        let mut words = HashSet::new();
        for line in dic.lines().skip(1) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (stem, flags) = line.split_once('/').unwrap_or((line, ""));
            let stem = stem.replace('’', "'");
            if stem.chars().count() > MAX_WORD || flags.chars().any(|f| skip.contains(&f)) {
                continue;
            }
            let flags: Vec<&Affix> = flags.chars().filter_map(|f| affixes.get(&f)).collect();
            let mut suffixed = Vec::new();
            for affix in flags.iter().filter(|a| !a.prefix) {
                for rule in &affix.rules {
                    if let Some(form) = rule.apply(&stem, false) {
                        suffixed.push((form, affix.cross));
                    }
                }
            }
            for affix in flags.iter().filter(|a| a.prefix) {
                for rule in &affix.rules {
                    if let Some(form) = rule.apply(&stem, true) {
                        words.insert(form);
                    }
                    if affix.cross {
                        for (form, cross) in &suffixed {
                            if *cross && let Some(both) = rule.apply(form, true) {
                                words.insert(both);
                            }
                        }
                    }
                }
            }
            words.extend(suffixed.into_iter().map(|(form, _)| form));
            words.insert(stem);
        }
        Dictionary { words, try_chars }
    }

    /// The built-in American English dictionary, read the first time it's asked for.
    pub fn english() -> &'static Dictionary {
        static ENGLISH: OnceLock<Dictionary> = OnceLock::new();
        ENGLISH.get_or_init(|| Dictionary::parse(AFF, DIC))
    }

    /// Whether `word` is spelled right: as written, or in lower case when it starts a
    /// sentence or is all capitals.
    pub fn check(&self, word: &str) -> bool {
        let word = word.replace('’', "'");
        if self.words.contains(&word) {
            return true;
        }
        let mut chars = word.chars();
        let Some(first) = chars.next() else { return true };
        let rest: String = chars.collect();
        let lower = word.to_lowercase();
        if first.is_uppercase() && rest.chars().all(|c| !c.is_uppercase()) {
            return self.words.contains(&lower);
        }
        if word.chars().all(|c| !c.is_lowercase()) {
            let capital: String = first.to_uppercase().chain(rest.to_lowercase().chars()).collect();
            return self.words.contains(&lower) || self.words.contains(&capital);
        }
        false
    }

    /// Up to `max` words that `word` may have been meant to be, nearest first, in its case.
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let word = word.replace('’', "'");
        let lower = word.to_lowercase();
        let capitalised = word.chars().next().is_some_and(char::is_uppercase);
        let all_caps =
            word.chars().filter(|c| c.is_alphabetic()).count() > 1 && word.chars().all(|c| !c.is_lowercase());
        let recase = |w: &str| -> String {
            if all_caps {
                w.to_uppercase()
            } else if capitalised {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_uppercase().chain(c).collect())
                    .unwrap_or_default()
            } else {
                w.to_owned()
            }
        };
        let known = |w: &str| self.words.contains(w) || self.words.contains(&recase(w));
        let mut out: Vec<String> = Vec::new();
        let push = |w: String, out: &mut Vec<String>| {
            let shown = recase(&w);
            if !out.contains(&shown) && shown != word {
                out.push(shown);
            }
        };
        let chars: Vec<char> = lower.chars().collect();
        let first = self.edits(&lower);
        // Nearest first: letters swapped, then changed, then one put in or taken out; a word
        // keeping the first letter before one that doesn't.
        let mut near: Vec<(u8, &String)> = first
            .iter()
            .filter(|(w, _)| known(w))
            .map(|(w, cost)| {
                (
                    *cost + u8::from(!w.starts_with(chars.first().copied().unwrap_or(' '))),
                    w,
                )
            })
            .collect();
        near.sort_by_key(|(cost, _)| *cost);
        for (_, w) in near {
            push(w.clone(), &mut out);
        }
        // Two words run together.
        for i in 2..chars.len().saturating_sub(1) {
            let (a, b): (String, String) = (
                chars.get(..i).unwrap_or_default().iter().collect(),
                chars.get(i..).unwrap_or_default().iter().collect(),
            );
            if a.chars().count() > 1 && self.words.contains(&a) && self.words.contains(&b) {
                push(format!("{a} {b}"), &mut out);
            }
        }
        if out.len() < max && chars.len() <= 20 {
            let mut seen = HashSet::new();
            let mut far: Vec<(u8, String)> = Vec::new();
            for (w, cost) in &first {
                for (w2, cost2) in self.edits(w) {
                    if seen.insert(w2.clone()) && known(&w2) {
                        far.push((cost + cost2, w2));
                    }
                }
                if far.len() >= max * 4 {
                    break;
                }
            }
            far.sort_by_key(|(cost, _)| *cost);
            for (_, w) in far {
                push(w, &mut out);
            }
        }
        out.truncate(max);
        out
    }

    /// Every string one edit from `word`, with how far it is: two letters swapped (0), one
    /// changed (1), one put in or taken out (2).
    fn edits(&self, word: &str) -> Vec<(String, u8)> {
        let chars: Vec<char> = word.chars().collect();
        let mut out = Vec::new();
        let letters: Vec<char> = self
            .try_chars
            .iter()
            .copied()
            .filter(|c| !c.is_uppercase())
            .collect();
        for i in 0..chars.len().saturating_sub(1) {
            let mut w = chars.clone();
            w.swap(i, i + 1);
            out.push((w.iter().collect(), 0));
        }
        for i in 0..chars.len() {
            for &c in &letters {
                let mut w = chars.clone();
                if let Some(slot) = w.get_mut(i) {
                    *slot = c;
                }
                out.push((w.iter().collect(), 1));
            }
        }
        for i in 0..=chars.len() {
            for &c in &letters {
                let mut w = chars.clone();
                w.insert(i, c);
                out.push((w.iter().collect(), 2));
            }
        }
        for i in 0..chars.len() {
            let mut w = chars.clone();
            w.remove(i);
            out.push((w.iter().collect(), 2));
        }
        out
    }
}

/// The words in `text` that are spelled wrong, as byte ranges, leaving out `ignored` words,
/// numbers, web and email addresses, and words with capitals in the middle (names of
/// things such as iPhone).
pub fn misspelled(text: &str, dict: &Dictionary, ignored: &HashSet<String>) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    for (start, token) in tokens(text) {
        if token.contains(['@', '/', '\\', '_', '.']) || token.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        if token.chars().count() > MAX_WORD || token.chars().count() < 2 {
            continue;
        }
        let inner_capital =
            token.chars().skip(1).any(char::is_uppercase) && token.chars().any(char::is_lowercase);
        if inner_capital || ignored.contains(&token.to_lowercase()) {
            continue;
        }
        // Each part of a hyphenated word on its own.
        let mut at = start;
        for part in token.split('-') {
            let range = at..at + part.len();
            at += part.len() + 1;
            let part = part.trim_matches('\'');
            if part.chars().count() >= 2 && !dict.check(part) && !dict.check(part.trim_end_matches("'s")) {
                out.push(range);
            }
        }
    }
    out
}

/// Runs of letters, digits, apostrophes, hyphens and the characters of addresses, with
/// their byte offsets; punctuation at the ends is left off.
fn tokens(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    let is_part =
        |c: char| c.is_alphanumeric() || matches!(c, '\'' | '’' | '-' | '@' | '/' | '\\' | '_' | '.' | ':');
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        match (start, is_part(c)) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                let raw = text.get(s..i).unwrap_or_default();
                let trimmed_start = raw.trim_start_matches(|c: char| !c.is_alphanumeric());
                let lead = raw.len() - trimmed_start.len();
                let word = trimmed_start.trim_end_matches(|c: char| !c.is_alphanumeric());
                if !word.is_empty() {
                    out.push((s + lead, word));
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing)]
    use super::*;

    fn small() -> Dictionary {
        let aff = "TRY esianrtolcdugmphbyfvkwz\nSFX S Y 2\nSFX S y ies [^aeiou]y\nSFX S 0 s [^y]\n\
                   PFX U Y 1\nPFX U 0 un .\nSFX D Y 2\nSFX D 0 d e\nSFX D 0 ed [^e]\n";
        let dic = "4\nhappy/U\nparty/S\nlike/DSU\nLondon\n";
        Dictionary::parse(aff, dic)
    }

    #[test]
    fn affixes_make_the_other_forms() {
        let d = small();
        for w in [
            "happy", "unhappy", "parties", "likes", "liked", "unliked", "unlikes", "London",
        ] {
            assert!(d.check(w), "{w}");
        }
        for w in ["partys", "london", "happys", "Unhappyed"] {
            assert!(!d.check(w), "{w}");
        }
        // A sentence's first word, and capitals throughout.
        assert!(d.check("Happy") && d.check("HAPPY") && d.check("LONDON"));
    }

    #[test]
    fn misspellings_are_found_and_suggested() {
        let d = small();
        let text = "Unhapy parites, see https://x.org or 42 iPhones; well-liked";
        let found: Vec<&str> = misspelled(text, &d, &HashSet::new())
            .into_iter()
            .map(|r| &text[r])
            .collect();
        assert_eq!(found, ["Unhapy", "parites", "see", "or", "well"]);
        assert_eq!(
            d.suggest("Unhapy", 3).first().map(String::as_str),
            Some("Unhappy")
        );
        assert!(d.suggest("parites", 5).contains(&"parties".to_owned()));
        let ignored = HashSet::from(["see".to_owned()]);
        assert!(!misspelled("see", &d, &ignored).iter().any(|_| true));
    }

    #[test]
    fn the_english_dictionary_knows_common_words() {
        let d = Dictionary::english();
        for w in [
            "the",
            "receive",
            "received",
            "receiving",
            "Organization",
            "children's",
            "doesn't",
            "Monday",
        ] {
            assert!(d.check(w), "{w}");
        }
        for w in ["recieve", "teh", "monday"] {
            assert!(!d.check(w), "{w}");
        }
        assert!(d.suggest("recieve", 5).contains(&"receive".to_owned()));
        assert!(d.suggest("teh", 5).contains(&"the".to_owned()));
    }
}
