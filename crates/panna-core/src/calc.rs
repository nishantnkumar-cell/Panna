//! Form calculations: the standard ones Acrobat writes (sum, product, average, minimum and
//! maximum of other fields, and simplified field notation such as `Price * Qty`) are
//! recognised in a field's calculate script and worked out here. The script itself is never
//! run, and any other script is ignored.

/// Terms in a simplified field notation expression at most.
const MAX_TERMS: usize = 200;
/// Fields one calculation names at most.
const MAX_NAMES: usize = 200;
/// How deep brackets nest at most.
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CalcOp {
    Sum,
    Product,
    Average,
    Min,
    Max,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Expr {
    Number(f64),
    /// A field's value, by its full name.
    Field(String),
    Neg(Box<Expr>),
    /// Two terms with `+`, `-`, `*` or `/` between them.
    Op(Box<Expr>, char, Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Formula {
    /// One operation over these fields' values (a group's name takes in all its fields).
    Simple(CalcOp, Vec<String>),
    /// Simplified field notation.
    Expression(Expr),
}

/// A field's value worked out from others.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Calculation {
    /// Its place in the form's calculation order: lower ones are worked out first.
    pub order: usize,
    pub formula: Formula,
}

impl Formula {
    /// Recognises a standard calculation in a field's calculate script.
    pub fn from_script(script: &str) -> Option<Self> {
        if let Some(start) = script.find("/** BVCALC")
            && let Some(len) = script.get(start..)?.find("EVCALC")
        {
            let notation = script.get(start + "/** BVCALC".len()..start + len)?;
            return parse(notation).map(Formula::Expression);
        }
        let start = script.find("AFSimple_Calculate(")? + "AFSimple_Calculate(".len();
        let strings = string_literals(script.get(start..)?);
        let mut strings = strings.into_iter();
        let op = match strings.next()?.as_str() {
            "SUM" => CalcOp::Sum,
            "PRD" => CalcOp::Product,
            "AVG" => CalcOp::Average,
            "MIN" => CalcOp::Min,
            "MAX" => CalcOp::Max,
            _ => return None,
        };
        let names: Vec<String> = strings.take(MAX_NAMES).collect();
        (!names.is_empty()).then_some(Formula::Simple(op, names))
    }

    /// The result, given each named field's numbers (several for a group); `None` when it
    /// can't be worked out, such as dividing by zero.
    pub fn evaluate(&self, values: &dyn Fn(&str) -> Vec<f64>) -> Option<f64> {
        let result = match self {
            Formula::Simple(op, names) => {
                let all: Vec<f64> = names.iter().flat_map(|n| values(n)).collect();
                match op {
                    CalcOp::Sum => all.iter().sum(),
                    CalcOp::Product => all.iter().product(),
                    CalcOp::Average if all.is_empty() => 0.0,
                    CalcOp::Average => all.iter().sum::<f64>() / all.len() as f64,
                    CalcOp::Min => all.iter().copied().reduce(f64::min).unwrap_or(0.0),
                    CalcOp::Max => all.iter().copied().reduce(f64::max).unwrap_or(0.0),
                }
            }
            Formula::Expression(expr) => eval(expr, values)?,
        };
        result.is_finite().then_some(result)
    }

    /// The fields it reads.
    pub fn inputs(&self) -> Vec<&str> {
        fn walk<'a>(e: &'a Expr, out: &mut Vec<&'a str>) {
            match e {
                Expr::Number(_) => {}
                Expr::Field(n) => out.push(n),
                Expr::Neg(a) => walk(a, out),
                Expr::Op(a, _, b) => {
                    walk(a, out);
                    walk(b, out);
                }
            }
        }
        match self {
            Formula::Simple(_, names) => names.iter().map(String::as_str).collect(),
            Formula::Expression(e) => {
                let mut out = Vec::new();
                walk(e, &mut out);
                out
            }
        }
    }
}

fn eval(e: &Expr, values: &dyn Fn(&str) -> Vec<f64>) -> Option<f64> {
    Some(match e {
        Expr::Number(v) => *v,
        Expr::Field(n) => values(n).iter().sum(),
        Expr::Neg(a) => -eval(a, values)?,
        Expr::Op(a, op, b) => {
            let (a, b) = (eval(a, values)?, eval(b, values)?);
            match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                _ if b == 0.0 => return None,
                _ => a / b,
            }
        }
    })
}

/// A number from what's typed or stored in a field: separators, currency signs and spaces
/// are left out, and anything else that isn't a number counts as 0, as in Acrobat.
pub fn number(text: &str) -> f64 {
    let kept: String = text
        .chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | 'e' | 'E'))
        .collect();
    kept.parse().unwrap_or(0.0)
}

/// A result as a field value: rounded off past ten decimal places, without trailing zeros.
pub fn value_text(v: f64) -> String {
    let rounded = (v * 1e10).round() / 1e10;
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    format!("{rounded}")
}

/// The quoted strings in a call's arguments, up to its closing bracket.
fn string_literals(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let mut current = String::new();
    let mut depth = 0usize;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(_), '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            (Some(q), c) if c == q => {
                quote = None;
                out.push(std::mem::take(&mut current));
                if out.len() > MAX_NAMES {
                    break;
                }
            }
            (Some(_), c) => {
                if current.len() < 512 {
                    current.push(c);
                }
            }
            (None, '"' | '\'') => quote = Some(c),
            (None, '(' | '[') => depth += 1,
            (None, ')' | ']') if depth == 0 => break,
            (None, ')' | ']') => depth -= 1,
            _ => {}
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Op(char),
    Open,
    Close,
}

fn tokens(s: &str) -> Option<Vec<Token>> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if out.len() > MAX_TERMS * 2 {
            return None;
        }
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '+' | '-' | '*' | '/' => {
                chars.next();
                out.push(Token::Op(c));
            }
            '(' => {
                chars.next();
                out.push(Token::Open);
            }
            ')' => {
                chars.next();
                out.push(Token::Close);
            }
            c if c.is_ascii_digit() || c == '.' => {
                let mut n = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() || d == '.' {
                        n.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push(Token::Number(n.parse().ok()?));
            }
            _ => {
                // A field name: anything up to a space, operator or bracket, with a backslash
                // keeping the character after it.
                let mut name = String::new();
                while let Some(&d) = chars.peek() {
                    if d == '\\' {
                        chars.next();
                        name.push(chars.next()?);
                    } else if d.is_whitespace() || "+-*/()".contains(d) {
                        break;
                    } else {
                        name.push(d);
                        chars.next();
                    }
                }
                out.push(Token::Name(name));
            }
        }
    }
    Some(out)
}

/// Parses simplified field notation, such as `(Price - Discount) * Qty`.
fn parse(notation: &str) -> Option<Expr> {
    let tokens = tokens(notation)?;
    let mut at = 0;
    let expr = sum(&tokens, &mut at, 0)?;
    (at == tokens.len()).then_some(expr)
}

fn sum(t: &[Token], at: &mut usize, depth: usize) -> Option<Expr> {
    let mut left = product(t, at, depth)?;
    while let Some(Token::Op(op @ ('+' | '-'))) = t.get(*at) {
        *at += 1;
        let right = product(t, at, depth)?;
        left = Expr::Op(Box::new(left), *op, Box::new(right));
    }
    Some(left)
}

fn product(t: &[Token], at: &mut usize, depth: usize) -> Option<Expr> {
    let mut left = unary(t, at, depth)?;
    while let Some(Token::Op(op @ ('*' | '/'))) = t.get(*at) {
        *at += 1;
        let right = unary(t, at, depth)?;
        left = Expr::Op(Box::new(left), *op, Box::new(right));
    }
    Some(left)
}

fn unary(t: &[Token], at: &mut usize, depth: usize) -> Option<Expr> {
    if depth > MAX_DEPTH {
        return None;
    }
    let token = t.get(*at)?.clone();
    *at += 1;
    match token {
        Token::Number(v) => Some(Expr::Number(v)),
        Token::Name(n) => Some(Expr::Field(n)),
        Token::Op('-') => Some(Expr::Neg(Box::new(unary(t, at, depth + 1)?))),
        Token::Op('+') => unary(t, at, depth + 1),
        Token::Open => {
            let inner = sum(t, at, depth + 1)?;
            match t.get(*at) {
                Some(Token::Close) => {
                    *at += 1;
                    Some(inner)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(name: &str) -> Vec<f64> {
        match name {
            "Price" => vec![12.5],
            "Qty" => vec![4.0],
            "Text 1" => vec![2.0],
            "items" => vec![1.0, 2.0, 3.0],
            _ => Vec::new(),
        }
    }

    #[test]
    fn simple_calculations_are_recognised_and_worked_out() {
        let f = Formula::from_script(r#"AFSimple_Calculate("SUM", new Array ("Price", "Qty"));"#).unwrap();
        assert_eq!(f.evaluate(&values), Some(16.5));
        let f = Formula::from_script(r#"AFSimple_Calculate("AVG", ["items"]);"#).unwrap();
        assert_eq!(f.evaluate(&values), Some(2.0));
        let f = Formula::from_script(r#"AFSimple_Calculate("MAX", "items, Qty");"#).unwrap();
        assert_eq!(f.inputs(), ["items, Qty"]);
        assert_eq!(Formula::from_script(r#"AFSimple_Calculate("POW", ["a"]);"#), None);
        assert_eq!(Formula::from_script("event.value = 1 + 2;"), None);
    }

    #[test]
    fn simplified_field_notation_is_worked_out() {
        let script = "/** BVCALC (Price - 2.5) * Qty / Text\\ 1 + -1 EVCALC **/ event.value = ...";
        let f = Formula::from_script(script).unwrap();
        assert_eq!(f.inputs(), ["Price", "Qty", "Text 1"]);
        assert_eq!(f.evaluate(&values), Some(19.0));
        let f = Formula::from_script("/** BVCALC Price / Missing EVCALC **/").unwrap();
        assert_eq!(f.evaluate(&values), None);
        assert_eq!(Formula::from_script("/** BVCALC (Price EVCALC **/"), None);
        assert_eq!(Formula::from_script("/** BVCALC Price Qty EVCALC **/"), None);
    }

    #[test]
    fn numbers_and_results_read_as_in_acrobat() {
        assert_eq!(number("$1,234.50"), 1234.5);
        assert_eq!(number(""), 0.0);
        assert_eq!(number("abc"), 0.0);
        assert_eq!(value_text(0.1 + 0.2), "0.3");
        assert_eq!(value_text(42.0), "42");
        assert_eq!(value_text(-0.0), "0");
    }
}
