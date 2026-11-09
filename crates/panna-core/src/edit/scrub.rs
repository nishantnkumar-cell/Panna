//! Finding redacted text again outside the page: in bookmark titles and comments, which
//! redacting areas of pages doesn't reach. Matching ignores case and treats any run of
//! whitespace as one space.

/// `s` in lower case with whitespace runs as single spaces, trimmed.
pub(crate) fn normalise(s: &str) -> String {
    s.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The phrases worth looking for, normalised and without repeats: at least two letters or
/// digits each, so a stray character redacted on its own doesn't take out every comment.
pub(crate) fn phrases<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for text in texts {
        let phrase = normalise(text);
        if phrase.chars().filter(|c| c.is_alphanumeric()).count() >= 2 && !out.contains(&phrase) {
            out.push(phrase);
        }
    }
    out
}

/// Whether `text` contains one of `phrases` (already normalised).
pub(crate) fn mentions(text: &str, phrases: &[String]) -> bool {
    let text = normalise(text);
    phrases.iter().any(|p| text.contains(p.as_str()))
}

/// `text` with whitespace runs as single spaces and every match of `phrases` (already
/// normalised) blacked out one block per character, or `None` when nothing matched.
pub(crate) fn black_out(text: &str, phrases: &[String]) -> Option<String> {
    let mut chars: Vec<char> = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect();
    // Lower case char by char, so positions line up with `chars`. Characters whose lower case
    // is several characters (rare in titles) are compared by their first.
    let lower: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let mut hit = vec![false; chars.len()];
    for phrase in phrases {
        let p: Vec<char> = phrase.chars().collect();
        if p.is_empty() || p.len() > lower.len() {
            continue;
        }
        for start in 0..=lower.len() - p.len() {
            if lower.get(start..start + p.len()) == Some(p.as_slice()) {
                hit.iter_mut().skip(start).take(p.len()).for_each(|h| *h = true);
            }
        }
    }
    if !hit.contains(&true) {
        return None;
    }
    for (c, h) in chars.iter_mut().zip(&hit) {
        if *h && !c.is_whitespace() {
            *c = '\u{2588}';
        }
    }
    Some(chars.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrases_are_found_whatever_their_case_and_spacing() {
        let found = phrases(["Acme  Ltd", "acme ltd", "x", " Jane\nDoe "]);
        assert_eq!(found, vec!["acme ltd".to_owned(), "jane doe".to_owned()]);
        assert!(mentions("Call ACME\tLtd. today", &found));
        assert!(!mentions("Call Acme today", &found));
        assert_eq!(
            black_out("Notes on  ACME Ltd and Jane Doe", &found).as_deref(),
            Some("Notes on ████ ███ and ████ ███")
        );
        assert_eq!(black_out("Nothing here", &found), None);
    }
}
