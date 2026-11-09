//! Kinds of private details found by their shape rather than by words, as Acrobat's Find Text
//! patterns for redaction: phone numbers, credit card numbers, social security numbers, email
//! addresses and dates.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Pattern {
    Phone,
    CreditCard,
    Ssn,
    Email,
    Date,
}

impl Pattern {
    pub const ALL: [Pattern; 5] = [
        Pattern::Phone,
        Pattern::CreditCard,
        Pattern::Ssn,
        Pattern::Email,
        Pattern::Date,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Pattern::Phone => "Phone numbers",
            Pattern::CreditCard => "Credit card numbers",
            Pattern::Ssn => "Social security numbers",
            Pattern::Email => "Email addresses",
            Pattern::Date => "Dates",
        }
    }
}

impl std::fmt::Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// One step of a shape.
#[derive(Clone, Copy)]
enum El {
    /// Between this many digits, as many as there are.
    Digits(usize, usize),
    /// One of these characters (a space also matches a line break).
    One(&'static str),
    /// One of these characters, or none.
    Maybe(&'static str),
    /// A month's name, in full or cut short.
    Month,
}

use El::{Digits as D, Maybe, Month, One};

const SEP: &str = " -.";

const PHONES: &[&[El]] = &[
    &[
        One("("),
        D(3, 3),
        One(")"),
        Maybe(" "),
        D(3, 3),
        One(SEP),
        D(4, 4),
    ],
    &[D(3, 3), One(SEP), D(3, 3), One(SEP), D(4, 4)],
    &[One("1"), One(SEP), D(3, 3), One(SEP), D(3, 3), One(SEP), D(4, 4)],
    &[
        One("+"),
        D(1, 3),
        One(SEP),
        D(1, 4),
        One(SEP),
        D(3, 4),
        One(SEP),
        D(3, 4),
    ],
    &[One("+"), D(1, 3), One(SEP), D(4, 5), One(SEP), D(4, 6)],
    &[
        One("+"),
        D(1, 3),
        One(SEP),
        One("("),
        D(1, 4),
        One(")"),
        Maybe(" "),
        D(3, 4),
        One(SEP),
        D(3, 4),
    ],
];

const CARDS: &[&[El]] = &[
    &[
        D(4, 4),
        One(" -"),
        D(4, 4),
        One(" -"),
        D(4, 4),
        One(" -"),
        D(1, 7),
    ],
    &[D(4, 4), One(" -"), D(6, 6), One(" -"), D(5, 5)],
    &[D(13, 19)],
];

const SSNS: &[&[El]] = &[&[D(3, 3), One("- "), D(2, 2), One("- "), D(4, 4)]];

const DATES: &[&[El]] = &[
    &[D(4, 4), One("/-."), D(1, 2), One("/-."), D(1, 2)],
    &[D(1, 2), One("/-."), D(1, 2), One("/-."), D(2, 4)],
    &[
        Month,
        Maybe("."),
        One(" "),
        D(1, 2),
        Maybe(","),
        One(" "),
        D(4, 4),
    ],
    &[
        D(1, 2),
        One(" "),
        Month,
        Maybe("."),
        Maybe(","),
        One(" "),
        D(4, 4),
    ],
];

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Where `chars` from `at` spells a month's name (or its first three letters, or Sept), the
/// end of it.
fn month(chars: &[char], at: usize) -> Option<usize> {
    let word: String = chars
        .get(at..)?
        .iter()
        .take_while(|c| c.is_alphabetic())
        .flat_map(|c| c.to_lowercase())
        .collect();
    let known = MONTHS
        .iter()
        .any(|m| *m == word || (word.len() == 3 && m.starts_with(&word)))
        || word == "sept";
    known.then(|| at + word.chars().count())
}

/// The end of `shape` matched at `at`, if it matches there.
fn shape_at(chars: &[char], at: usize, shape: &[El]) -> Option<usize> {
    let mut i = at;
    let is = |i: usize, set: &str| {
        chars
            .get(i)
            .is_some_and(|&c| set.contains(c) || (c == '\n' && set.contains(' ')))
    };
    for el in shape {
        match *el {
            D(min, max) => {
                let run = chars.get(i..)?.iter().take_while(|c| c.is_ascii_digit()).count();
                if run < min || run > max {
                    return None;
                }
                i += run;
            }
            One(set) => {
                if !is(i, set) {
                    return None;
                }
                i += 1;
            }
            Maybe(set) => {
                if is(i, set) {
                    i += 1;
                }
            }
            Month => i = month(chars, i)?,
        }
    }
    Some(i)
}

/// The digits in `chars`, as numbers group by group.
fn groups(chars: &[char]) -> Vec<(u32, usize)> {
    let mut out = Vec::new();
    let mut run = String::new();
    for c in chars.iter().chain(std::iter::once(&' ')) {
        if c.is_ascii_digit() {
            run.push(*c);
        } else if !run.is_empty() {
            out.push((run.parse().unwrap_or(u32::MAX), run.len()));
            run.clear();
        }
    }
    out
}

/// The Luhn check that card numbers pass.
fn luhn(digits: &[u32]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                let d = d * 2;
                if d > 9 { d - 9 } else { d }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// Whether what matched a shape really is one of `pattern`.
fn plausible(pattern: Pattern, found: &[char]) -> bool {
    match pattern {
        Pattern::CreditCard => {
            let digits: Vec<u32> = found.iter().filter_map(|c| c.to_digit(10)).collect();
            (13..=19).contains(&digits.len()) && luhn(&digits)
        }
        Pattern::Ssn => {
            let g = groups(found);
            // Never issued: area 000, 666 or 9xx, group 00, serial 0000.
            matches!(g.as_slice(), [(a, _), (b, _), (c, _)] if *a != 0 && *a != 666 && *a < 900 && *b != 0 && *c != 0)
        }
        Pattern::Date => {
            let seps: Vec<char> = found.iter().copied().filter(|c| "/-.".contains(*c)).collect();
            if found.iter().any(|c| c.is_alphabetic()) {
                return groups(found).first().is_some_and(|(d, _)| (1..=31).contains(d));
            }
            if seps.windows(2).any(|w| w.first() != w.get(1)) {
                return false;
            }
            match groups(found).as_slice() {
                [(y, 4), (m, _), (d, _)] => {
                    (1000..=2999).contains(y) && (1..=12).contains(m) && (1..=31).contains(d)
                }
                [(a, _), (b, _), (_, len)] if *len == 2 || *len == 4 => {
                    let day = 1..=31;
                    let month = 1..=12;
                    (month.contains(a) && day.contains(b)) || (day.contains(a) && month.contains(b))
                }
                _ => false,
            }
        }
        Pattern::Phone | Pattern::Email => true,
    }
}

/// Email addresses: a name, @, and a domain with a dot in it.
fn emails(chars: &[char]) -> Vec<Range<usize>> {
    let local = |c: char| c.is_alphanumeric() || "._%+-".contains(c);
    let domain = |c: char| c.is_alphanumeric() || ".-".contains(c);
    let mut out: Vec<Range<usize>> = Vec::new();
    for (at, _) in chars.iter().enumerate().filter(|(_, c)| **c == '@') {
        if out.last().is_some_and(|r| r.end > at) {
            continue;
        }
        let mut start = at;
        while start > 0 && chars.get(start - 1).is_some_and(|c| local(*c)) {
            start -= 1;
        }
        // A name doesn't start with a dot (the end of a sentence, say).
        while chars.get(start).is_some_and(|c| *c == '.') && start < at {
            start += 1;
        }
        let mut end = at + 1;
        while chars.get(end).is_some_and(|c| domain(*c)) {
            end += 1;
        }
        while end > at + 1 && chars.get(end - 1).is_some_and(|c| ".-".contains(*c)) {
            end -= 1;
        }
        let host: String = chars.get(at + 1..end).unwrap_or_default().iter().collect();
        let tld_ok = host.rsplit_once('.').is_some_and(|(name, tld)| {
            !name.is_empty() && tld.chars().count() >= 2 && tld.chars().all(char::is_alphabetic)
        });
        if start < at && tld_ok {
            out.push(start..end);
        }
    }
    out
}

/// Every place in `chars` with the shape of `pattern`, as character ranges, in order.
pub(crate) fn find(chars: &[char], pattern: Pattern) -> Vec<Range<usize>> {
    let shapes = match pattern {
        Pattern::Email => return emails(chars),
        Pattern::Phone => PHONES,
        Pattern::CreditCard => CARDS,
        Pattern::Ssn => SSNS,
        Pattern::Date => DATES,
    };
    let word = |i: usize| chars.get(i).is_some_and(|c| c.is_alphanumeric());
    let mut out = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let starts_word = at == 0 || !word(at - 1);
        let found = starts_word
            .then(|| {
                shapes
                    .iter()
                    .filter_map(|shape| shape_at(chars, at, shape))
                    .filter(|&end| !word(end))
                    .filter(|&end| plausible(pattern, chars.get(at..end).unwrap_or_default()))
                    .max()
            })
            .flatten();
        match found {
            Some(end) => {
                out.push(at..end);
                at = end;
            }
            None => at += 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str, pattern: Pattern) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        find(&chars, pattern)
            .into_iter()
            .map(|r| chars[r].iter().collect())
            .collect()
    }

    #[test]
    fn phone_numbers() {
        assert_eq!(
            found(
                "Call (555) 123-4567, 555.123.4567 or +44 20 7946 0958; not 12345 or 555-12-3456.",
                Pattern::Phone
            ),
            ["(555) 123-4567", "555.123.4567", "+44 20 7946 0958"]
        );
    }

    #[test]
    fn card_numbers_must_pass_the_check() {
        assert_eq!(
            found(
                "Visa 4111 1111 1111 1111, Amex 3782-822463-10005, typo 4111 1111 1111 1112.",
                Pattern::CreditCard
            ),
            ["4111 1111 1111 1111", "3782-822463-10005"]
        );
    }

    #[test]
    fn social_security_numbers() {
        assert_eq!(
            found("SSN 123-45-6789, 000-12-3456, phone 555-123-4567", Pattern::Ssn),
            ["123-45-6789"]
        );
    }

    #[test]
    fn email_addresses() {
        assert_eq!(
            found("Mail asha.rao@example.co.in. Or @handle, or a@b.", Pattern::Email),
            ["asha.rao@example.co.in"]
        );
    }

    #[test]
    fn dates() {
        assert_eq!(
            found(
                "Signed 2026-11-09, due 9/12/2026 and March 5, 2026 or 5 Sept 2026; not 13/13/2026 or 1-2.3-2026, version 1.2.3",
                Pattern::Date
            ),
            ["2026-11-09", "9/12/2026", "March 5, 2026", "5 Sept 2026"]
        );
    }
}
