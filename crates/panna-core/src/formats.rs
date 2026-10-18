//! Field formats: numbers, percentages, dates and special numbers (postcodes, phone numbers).
//!
//! PDF forms set these with JavaScript calls such as `AFNumber_Format(2, 0, 0, 0, "$", true)`.
//! Panna never runs scripts; it only recognises these standard calls by name and applies the
//! same formatting itself.

/// How a text field's value is shown and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Format {
    Number {
        decimals: u8,
        /// 0 `1,234.56`, 1 `1234.56`, 2 `1.234,56`, 3 `1234,56`.
        separators: u8,
        currency: String,
        /// The currency symbol goes before the number.
        prepend: bool,
    },
    Percent {
        decimals: u8,
        separators: u8,
    },
    /// A date pattern such as `mm/dd/yyyy`.
    Date(String),
    /// 0 ZIP code, 1 ZIP+4, 2 phone number, 3 social security number.
    Special(u8),
}

/// Patterns for `AFDate_Format(n)`, by index.
const DATE_FORMATS: [&str; 12] = [
    "m/d",
    "m/d/yy",
    "mm/dd/yy",
    "mm/yy",
    "d-mmm",
    "d-mmm-yy",
    "dd-mmm-yy",
    "yy-mm-dd",
    "mmm-yy",
    "mmmm-yy",
    "mmm d, yyyy",
    "mmmm d, yyyy",
];

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// The arguments of the first call to `function` in a script: numbers, strings and booleans,
/// as text.
fn call_args(script: &str, function: &str) -> Option<Vec<String>> {
    let start = script.find(&format!("{function}("))? + function.len() + 1;
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = script.get(start..)?.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
                let _ = q;
            }
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            (None, '"' | '\'') => quote = Some(c),
            (None, ',') => args.push(std::mem::take(&mut current).trim().to_owned()),
            (None, ')') => {
                let last = current.trim().to_owned();
                if !last.is_empty() || !args.is_empty() {
                    args.push(last);
                }
                return Some(args);
            }
            (None, c) => current.push(c),
        }
        if args.len() > 16 || current.len() > 256 {
            return None;
        }
    }
    None
}

impl Format {
    /// Recognises a standard format call in a field's format or keystroke script.
    pub fn from_script(script: &str) -> Option<Self> {
        let num = |args: &[String], i: usize| -> u8 {
            args.get(i)
                .and_then(|a| a.parse::<f64>().ok())
                .map_or(0, |v| v.clamp(0.0, 10.0) as u8)
        };
        for f in ["AFNumber_Format", "AFNumber_Keystroke"] {
            if let Some(args) = call_args(script, f) {
                return Some(Format::Number {
                    decimals: num(&args, 0),
                    separators: num(&args, 1).min(3),
                    currency: args.get(4).cloned().unwrap_or_default(),
                    prepend: args.get(5).is_none_or(|a| a != "false"),
                });
            }
        }
        for f in ["AFPercent_Format", "AFPercent_Keystroke"] {
            if let Some(args) = call_args(script, f) {
                return Some(Format::Percent {
                    decimals: num(&args, 0),
                    separators: num(&args, 1).min(3),
                });
            }
        }
        for f in ["AFDate_FormatEx", "AFDate_KeystrokeEx"] {
            if let Some(args) = call_args(script, f) {
                let pattern = args.first()?.clone();
                // Patterns with times aren't supported yet.
                if pattern.contains(['h', 'H', 'M', 's', 't']) {
                    return None;
                }
                return Some(Format::Date(pattern));
            }
        }
        for f in ["AFDate_Format", "AFDate_Keystroke"] {
            if let Some(args) = call_args(script, f) {
                let pattern = DATE_FORMATS.get(usize::from(num(&args, 0)))?;
                return Some(Format::Date((*pattern).to_owned()));
            }
        }
        for f in ["AFSpecial_Format", "AFSpecial_Keystroke"] {
            if let Some(args) = call_args(script, f) {
                return Some(Format::Special(num(&args, 0).min(3)));
            }
        }
        None
    }

    /// Whether a typed character can be part of a value.
    pub fn allows(&self, c: char) -> bool {
        match self {
            Format::Number { currency, .. } => {
                c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | ' ') || currency.contains(c)
            }
            Format::Percent { .. } => c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | ' ' | '%'),
            Format::Special(_) => c.is_ascii_digit() || matches!(c, '-' | '(' | ')' | ' ' | '.'),
            Format::Date(_) => !c.is_control(),
        }
    }

    /// Whether `raw` is a valid value (an empty field always is).
    pub fn accepts(&self, raw: &str) -> bool {
        raw.trim().is_empty() || self.display(raw).is_some()
    }

    /// The value as the form shows it, or `None` when it isn't valid.
    pub fn display(&self, raw: &str) -> Option<String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Some(String::new());
        }
        match self {
            Format::Number {
                decimals,
                separators,
                currency,
                prepend,
            } => {
                let v = parse_number(raw, *separators, currency)?;
                let n = group(v, *decimals, *separators);
                Some(match (currency.is_empty(), prepend) {
                    (true, _) => n,
                    (false, true) => match n.strip_prefix('-') {
                        Some(abs) => format!("-{currency}{abs}"),
                        None => format!("{currency}{n}"),
                    },
                    (false, false) => format!("{n} {currency}"),
                })
            }
            Format::Percent { decimals, separators } => {
                // A typed "12%" means 12 percent; a bare 0.12 is the fraction.
                let v = match raw.strip_suffix('%') {
                    Some(p) => parse_number(p.trim(), *separators, "")? / 100.0,
                    None => parse_number(raw, *separators, "")?,
                };
                Some(format!("{}%", group(v * 100.0, *decimals, *separators)))
            }
            Format::Date(pattern) => {
                let (y, m, d) = parse_date(raw, pattern)?;
                Some(format_date(pattern, y, m, d))
            }
            Format::Special(kind) => {
                let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
                let d = |r: std::ops::Range<usize>| digits.get(r).unwrap_or_default();
                match (kind, digits.len()) {
                    (0, 5) => Some(digits),
                    (1, 9) => Some(format!("{}-{}", d(0..5), d(5..9))),
                    (2, 10) => Some(format!("({}) {}-{}", d(0..3), d(3..6), d(6..10))),
                    (2, 7) => Some(format!("{}-{}", d(0..3), d(3..7))),
                    (3, 9) => Some(format!("{}-{}-{}", d(0..3), d(3..5), d(5..9))),
                    _ => None,
                }
            }
        }
    }

    /// The value saved in the field: numbers with a plain `.` decimal point, as Acrobat stores
    /// them, and other formats as shown.
    pub fn saved(&self, raw: &str) -> String {
        match self {
            Format::Number {
                separators, currency, ..
            } => parse_number(raw.trim(), *separators, currency)
                .map_or_else(|| raw.to_owned(), |v| v.to_string()),
            Format::Percent { separators, .. } => {
                let raw = raw.trim();
                let v = match raw.strip_suffix('%') {
                    Some(p) => parse_number(p.trim(), *separators, "").map(|v| v / 100.0),
                    None => parse_number(raw, *separators, ""),
                };
                v.map_or_else(|| raw.to_owned(), |v| v.to_string())
            }
            _ => self.display(raw).unwrap_or_else(|| raw.to_owned()),
        }
    }
}

/// Reads a number written with the given separator style, ignoring grouping marks, spaces and
/// the currency symbol. Either `.` or `,` is taken as the decimal mark when only one is used.
fn parse_number(raw: &str, separators: u8, currency: &str) -> Option<f64> {
    let mut s: String = raw
        .replace(currency, "")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if currency.is_empty() || !raw.contains(currency) {
        s = s
            .trim_matches(|c: char| !(c.is_ascii_digit() || matches!(c, '-' | '.' | ',')))
            .to_owned();
    }
    let comma_decimal = separators >= 2;
    let (group, decimal) = if comma_decimal { ('.', ',') } else { (',', '.') };
    let normal: String = if s.contains(decimal) || !s.contains(group) || s.matches(group).count() > 1 {
        s.chars()
            .filter(|c| *c != group)
            .map(|c| if c == decimal { '.' } else { c })
            .collect()
    } else {
        // Only the other mark appears once, e.g. "3,5" typed into a 1,234.56 field: when it
        // isn't followed by exactly three digits it is a decimal point.
        let after = s.rsplit(group).next().map_or(0, str::len);
        if after == 3 {
            s.replace(group, "")
        } else {
            s.replace(group, ".")
        }
    };
    if normal.is_empty() || normal.chars().filter(|c| *c == '.').count() > 1 {
        return None;
    }
    normal
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && v.abs() < 1e15)
}

/// A number rounded to `decimals`, with thousands grouped in the separator style.
fn group(v: f64, decimals: u8, separators: u8) -> String {
    let fixed = format!("{:.*}", usize::from(decimals), v.abs());
    let (int, frac) = fixed.split_once('.').unwrap_or((&fixed, ""));
    let (group, decimal) = match separators {
        0 => (Some(','), '.'),
        1 => (None, '.'),
        2 => (Some('.'), ','),
        _ => (None, ','),
    };
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0
            && (int.len() - i) % 3 == 0
            && let Some(g) = group
        {
            out.push(g);
        }
        out.push(c);
    }
    if !frac.is_empty() {
        out.push(decimal);
        out.push_str(frac);
    }
    // No "-0.00".
    if v < 0.0 && out.chars().any(|c| c.is_ascii_digit() && c != '0') {
        out.insert(0, '-');
    }
    out
}

/// The date parts a pattern asks for, in order: 'y', 'm' or 'd'.
fn parts(pattern: &str) -> Vec<char> {
    let mut out = Vec::new();
    let mut last = None;
    for c in pattern.chars() {
        if matches!(c, 'y' | 'm' | 'd') && last != Some(c) {
            out.push(c);
        }
        last = Some(c);
    }
    out
}

/// Reads a date typed in the pattern's order, allowing any separators and month names.
fn parse_date(raw: &str, pattern: &str) -> Option<(i32, u32, u32)> {
    let order = parts(pattern);
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    for c in raw.chars() {
        let same = current
            .chars()
            .last()
            .is_none_or(|l| l.is_ascii_digit() == c.is_ascii_digit());
        if c.is_alphanumeric() && same {
            current.push(c);
        } else {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            if c.is_alphanumeric() {
                current.push(c);
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    if tokens.len() != order.len() {
        return None;
    }
    let (mut y, mut m, mut d) = (2000, 1, 1);
    for (part, token) in order.iter().zip(&tokens) {
        match part {
            'm' => {
                m = match token.parse::<u32>() {
                    Ok(n) => n,
                    Err(_) => {
                        let lower = token.to_lowercase();
                        let found = MONTHS
                            .iter()
                            .position(|name| lower.len() >= 3 && name.to_lowercase().starts_with(&lower))?;
                        u32::try_from(found).ok()? + 1
                    }
                }
            }
            'd' => d = token.parse().ok()?,
            _ => {
                let n: i32 = token.parse().ok()?;
                y = match token.len() {
                    // Two-digit years: 50 and up are the 1900s, as in Acrobat.
                    1 | 2 if n < 50 => 2000 + n,
                    1 | 2 => 1900 + n,
                    4 => n,
                    _ => return None,
                };
            }
        }
    }
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        _ => return None,
    };
    (1..=days).contains(&d).then_some((y, m, d))
}

fn format_date(pattern: &str, y: i32, m: u32, d: u32) -> String {
    let month = MONTHS.get(m as usize - 1).copied().unwrap_or("");
    let mut out = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let run = chars
            .get(i..)
            .map_or(0, |rest| rest.iter().take_while(|x| **x == c).count());
        match (c, run) {
            ('y', 4..) => out.push_str(&format!("{y:04}")),
            ('y', _) => out.push_str(&format!("{:02}", y.rem_euclid(100))),
            ('m', 1) => out.push_str(&m.to_string()),
            ('m', 2) => out.push_str(&format!("{m:02}")),
            ('m', 3) => out.push_str(month.get(..3).unwrap_or(month)),
            ('m', _) => out.push_str(month),
            ('d', 1) => out.push_str(&d.to_string()),
            ('d', _) => out.push_str(&format!("{d:02}")),
            _ => {
                for _ in 0..run {
                    out.push(c);
                }
            }
        }
        i += run.max(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_calls_are_recognised_without_running_them() {
        assert_eq!(
            Format::from_script("AFNumber_Format(2, 0, 0, 0, \"$\", true);"),
            Some(Format::Number {
                decimals: 2,
                separators: 0,
                currency: "$".into(),
                prepend: true
            })
        );
        assert_eq!(
            Format::from_script("AFDate_FormatEx(\"dd/mm/yyyy\");"),
            Some(Format::Date("dd/mm/yyyy".into()))
        );
        assert_eq!(
            Format::from_script("AFDate_Format(2)"),
            Some(Format::Date("mm/dd/yy".into()))
        );
        assert_eq!(
            Format::from_script("AFSpecial_Keystroke(2);"),
            Some(Format::Special(2))
        );
        assert_eq!(Format::from_script("app.alert('hi')"), None);
        assert_eq!(Format::from_script("AFDate_FormatEx(\"h:MM tt\")"), None);
    }

    #[test]
    fn numbers_are_grouped_and_rounded() {
        let usd = Format::Number {
            decimals: 2,
            separators: 0,
            currency: "$".into(),
            prepend: true,
        };
        assert_eq!(usd.display("1234.5").as_deref(), Some("$1,234.50"));
        assert_eq!(usd.display("$1,234.50").as_deref(), Some("$1,234.50"));
        assert_eq!(usd.display("-3").as_deref(), Some("-$3.00"));
        assert_eq!(usd.display("abc"), None);
        assert_eq!(usd.saved("$1,234.50"), "1234.5");
        let eu = Format::Number {
            decimals: 2,
            separators: 2,
            currency: "€".into(),
            prepend: false,
        };
        assert_eq!(eu.display("1234,5").as_deref(), Some("1.234,50 €"));
        let pct = Format::Percent {
            decimals: 1,
            separators: 0,
        };
        assert_eq!(pct.display("12.5%").as_deref(), Some("12.5%"));
        assert_eq!(pct.display("0.125").as_deref(), Some("12.5%"));
    }

    #[test]
    fn dates_are_read_in_the_pattern_order() {
        let uk = Format::Date("dd/mm/yyyy".into());
        assert_eq!(uk.display("3/4/26").as_deref(), Some("03/04/2026"));
        assert_eq!(uk.display("31/2/2026"), None);
        assert_eq!(uk.display("29-02-2024").as_deref(), Some("29/02/2024"));
        let long = Format::Date("mmmm d, yyyy".into());
        assert_eq!(long.display("oct 17 2026").as_deref(), Some("October 17, 2026"));
        assert!(long.accepts("") && !long.accepts("someday"));
    }

    #[test]
    fn special_numbers() {
        assert_eq!(
            Format::Special(2).display("5551234567").as_deref(),
            Some("(555) 123-4567")
        );
        assert_eq!(
            Format::Special(1).display("12345 6789").as_deref(),
            Some("12345-6789")
        );
        assert_eq!(Format::Special(0).display("123"), None);
    }
}
