//! A small formula evaluator for cell.calc(): arithmetic, comparisons,
//! text joining, cell and range references (also to other sheets) and the
//! common functions. Results are returned as text, like Excel shows them in
//! the General format.

use umya_spreadsheet::CellRawValue;

use super::xlsx::{Book, column_number, fmt_num, parse_ref};
use crate::{Error, Result};

#[derive(Clone, Debug)]
enum V {
    Num(f64),
    Str(String),
    Bool(bool),
    Empty,
    Err(String),
    Range(Vec<Vec<V>>),
}

impl V {
    fn num(&self) -> std::result::Result<f64, String> {
        match self {
            V::Num(n) => Ok(*n),
            V::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            V::Empty => Ok(0.0),
            V::Str(s) => s.trim().parse::<f64>().map_err(|_| "#VALUE!".to_string()),
            V::Err(e) => Err(e.clone()),
            V::Range(r) => r.first().and_then(|row| row.first()).map(|v| v.num()).unwrap_or(Ok(0.0)),
        }
    }
    fn text(&self) -> std::result::Result<String, String> {
        match self {
            V::Num(n) => Ok(super::numfmt::general(*n)),
            V::Str(s) => Ok(s.clone()),
            V::Bool(b) => Ok(if *b { "TRUE".into() } else { "FALSE".into() }),
            V::Empty => Ok(String::new()),
            V::Err(e) => Err(e.clone()),
            V::Range(r) => r.first().and_then(|row| row.first()).map(|v| v.text()).unwrap_or(Ok(String::new())),
        }
    }
    fn truthy(&self) -> std::result::Result<bool, String> {
        match self {
            V::Bool(b) => Ok(*b),
            V::Str(s) if s.eq_ignore_ascii_case("true") => Ok(true),
            V::Str(s) if s.eq_ignore_ascii_case("false") => Ok(false),
            V::Str(_) => Err("#VALUE!".into()),
            other => other.num().map(|n| n != 0.0),
        }
    }
    /// The values of a range (or the value itself).
    fn flat(&self) -> Vec<V> {
        match self {
            V::Range(r) => r.iter().flatten().cloned().collect(),
            v => vec![v.clone()],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ref(String),
    Name(String),
    Op(String),
    LParen,
    RParen,
    Comma,
}

fn tokenize(s: &str) -> Result<Vec<Tok>> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch.is_ascii_digit() || (ch == '.' && i + 1 < c.len() && c[i + 1].is_ascii_digit()) {
            let st = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') {
                i += 1;
            }
            if i < c.len() && (c[i] == 'e' || c[i] == 'E') {
                i += 1;
                if i < c.len() && (c[i] == '+' || c[i] == '-') {
                    i += 1;
                }
                while i < c.len() && c[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let t: String = c[st..i].iter().collect();
            out.push(Tok::Num(t.parse().map_err(|_| Error::msg(format!("invalid number {t}")))?));
        } else if ch == '"' {
            let mut t = String::new();
            i += 1;
            while i < c.len() {
                if c[i] == '"' {
                    if i + 1 < c.len() && c[i + 1] == '"' {
                        t.push('"');
                        i += 2;
                        continue;
                    }
                    break;
                }
                t.push(c[i]);
                i += 1;
            }
            i += 1;
            out.push(Tok::Str(t));
        } else if ch == '\'' {
            // 'Sheet name'!A1
            let mut t = String::new();
            i += 1;
            while i < c.len() && c[i] != '\'' {
                t.push(c[i]);
                i += 1;
            }
            i += 1;
            if i < c.len() && c[i] == '!' {
                i += 1;
                let st = i;
                while i < c.len() && (c[i].is_alphanumeric() || c[i] == '$' || c[i] == ':') {
                    i += 1;
                }
                let r: String = c[st..i].iter().collect();
                out.push(Tok::Ref(format!("{t}!{r}")));
            }
        } else if ch.is_alphabetic() || ch == '_' || ch == '$' {
            let st = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '.' || c[i] == '$' || c[i] == ':' || c[i] == '!') {
                i += 1;
            }
            let t: String = c[st..i].iter().collect();
            if (i < c.len() && c[i] == '(') || t.eq_ignore_ascii_case("TRUE") || t.eq_ignore_ascii_case("FALSE") {
                out.push(Tok::Name(t.to_uppercase()));
            } else {
                out.push(Tok::Ref(t));
            }
        } else if ch == '(' {
            out.push(Tok::LParen);
            i += 1;
        } else if ch == ')' {
            out.push(Tok::RParen);
            i += 1;
        } else if ch == ',' {
            out.push(Tok::Comma);
            i += 1;
        } else {
            let two: String = c[i..(i + 2).min(c.len())].iter().collect();
            if ["<=", ">=", "<>"].contains(&two.as_str()) {
                out.push(Tok::Op(two));
                i += 2;
            } else if "+-*/^&=<>%".contains(ch) {
                out.push(Tok::Op(ch.to_string()));
                i += 1;
            } else {
                return Err(Error::msg(format!("formula: unexpected character '{ch}'")));
            }
        }
    }
    Ok(out)
}

struct Ev<'a> {
    b: &'a Book,
    sheet: String,
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

const MAX_DEPTH: usize = 64;

impl<'a> Ev<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn op_is(&self, ops: &[&str]) -> Option<String> {
        match self.peek() {
            Some(Tok::Op(o)) if ops.contains(&o.as_str()) => Some(o.clone()),
            _ => None,
        }
    }

    fn expr(&mut self) -> Result<V> {
        let mut l = self.concat()?;
        while let Some(op) = self.op_is(&["=", "<>", "<", ">", "<=", ">="]) {
            self.pos += 1;
            let r = self.concat()?;
            l = compare(&l, &r, &op);
        }
        Ok(l)
    }
    fn concat(&mut self) -> Result<V> {
        let mut l = self.add()?;
        while self.op_is(&["&"]).is_some() {
            self.pos += 1;
            let r = self.add()?;
            l = match (l.text(), r.text()) {
                (Ok(a), Ok(b)) => V::Str(a + &b),
                (Err(e), _) | (_, Err(e)) => V::Err(e),
            };
        }
        Ok(l)
    }
    fn add(&mut self) -> Result<V> {
        let mut l = self.mul()?;
        while let Some(op) = self.op_is(&["+", "-"]) {
            self.pos += 1;
            let r = self.mul()?;
            l = arith(&l, &r, |a, b| if op == "+" { Ok(a + b) } else { Ok(a - b) });
        }
        Ok(l)
    }
    fn mul(&mut self) -> Result<V> {
        let mut l = self.pow()?;
        while let Some(op) = self.op_is(&["*", "/"]) {
            self.pos += 1;
            let r = self.pow()?;
            l = arith(&l, &r, |a, b| {
                if op == "*" {
                    Ok(a * b)
                } else if b == 0.0 {
                    Err("#DIV/0!".into())
                } else {
                    Ok(a / b)
                }
            });
        }
        Ok(l)
    }
    fn pow(&mut self) -> Result<V> {
        let mut l = self.unary()?;
        while self.op_is(&["^"]).is_some() {
            self.pos += 1;
            let r = self.unary()?;
            l = arith(&l, &r, |a, b| Ok(a.powf(b)));
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<V> {
        if let Some(op) = self.op_is(&["-", "+"]) {
            self.pos += 1;
            let v = self.unary()?;
            return Ok(if op == "-" { arith(&V::Num(0.0), &v, |a, b| Ok(a - b)) } else { v });
        }
        let v = self.primary()?;
        if self.op_is(&["%"]).is_some() {
            self.pos += 1;
            return Ok(arith(&v, &V::Num(100.0), |a, b| Ok(a / b)));
        }
        Ok(v)
    }
    fn primary(&mut self) -> Result<V> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(V::Num(n)),
            Some(Tok::Str(s)) => Ok(V::Str(s)),
            Some(Tok::LParen) => {
                let v = self.expr()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(v),
                    _ => Err(Error::msg("formula: missing )")),
                }
            }
            Some(Tok::Ref(r)) => self.reference(&r),
            Some(Tok::Name(n)) if n == "TRUE" || n == "FALSE" => {
                if self.peek() == Some(&Tok::LParen) {
                    self.pos += 1;
                    self.next();
                }
                Ok(V::Bool(n == "TRUE"))
            }
            Some(Tok::Name(n)) => {
                self.next(); // (
                let mut args = Vec::new();
                if self.peek() != Some(&Tok::RParen) {
                    loop {
                        if matches!(self.peek(), Some(Tok::Comma) | Some(Tok::RParen)) {
                            args.push(V::Empty);
                        } else {
                            args.push(self.expr()?);
                        }
                        match self.next() {
                            Some(Tok::Comma) => continue,
                            Some(Tok::RParen) => break,
                            _ => return Err(Error::msg("formula: missing )")),
                        }
                    }
                } else {
                    self.next();
                }
                Ok(call(&n, args))
            }
            t => Err(Error::msg(format!("formula: unexpected {t:?}"))),
        }
    }

    fn reference(&mut self, r: &str) -> Result<V> {
        let (sheet, r) = match r.rsplit_once('!') {
            Some((s, r)) => (s.trim_matches('\'').to_string(), r.to_string()),
            None => (self.sheet.clone(), r.to_string()),
        };
        let r = r.replace('$', "").to_uppercase();
        let (a, b) = match r.split_once(':') {
            Some((a, b)) => (a.to_string(), b.to_string()),
            None => (r.clone(), r.clone()),
        };
        let ws = self.b.sheet(&sheet)?;
        // Whole columns (A:A) end at the last used row.
        let corner = |s: &str, last: bool| -> Result<(u32, u32)> {
            if s.chars().all(|c| c.is_ascii_alphabetic()) {
                let col = column_number(s)?;
                let row = if last { ws.highest_row().max(1) } else { 1 };
                return Ok((col, row));
            }
            parse_ref(s)
        };
        let (c1, r1) = corner(&a, false)?;
        let (c2, r2) = corner(&b, true)?;
        let mut rows = Vec::new();
        for row in r1.min(r2)..=r1.max(r2) {
            let mut vals = Vec::new();
            for col in c1.min(c2)..=c1.max(c2) {
                vals.push(self.cell(&sheet, col, row)?);
            }
            rows.push(vals);
        }
        if rows.len() == 1 && rows[0].len() == 1 && a == b {
            return Ok(rows.remove(0).remove(0));
        }
        Ok(V::Range(rows))
    }

    fn cell(&mut self, sheet: &str, col: u32, row: u32) -> Result<V> {
        let ws = self.b.sheet(sheet)?;
        let Some(c) = ws.cell((col, row)) else { return Ok(V::Empty) };
        if c.is_formula() && matches!(c.raw_value(), CellRawValue::Empty) {
            if self.depth >= MAX_DEPTH {
                return Ok(V::Err("#REF!".into()));
            }
            let mut ev = Ev { b: self.b, sheet: sheet.to_string(), toks: tokenize(c.formula())?, pos: 0, depth: self.depth + 1 };
            return ev.expr();
        }
        Ok(match c.raw_value() {
            CellRawValue::Numeric(n) => V::Num(*n),
            CellRawValue::Bool(b) => V::Bool(*b),
            CellRawValue::Empty => V::Empty,
            CellRawValue::Error(e) => V::Err(e.to_string()),
            other => {
                let s = other.to_string();
                if s.is_empty() { V::Empty } else { V::Str(s) }
            }
        })
    }
}

fn arith(a: &V, b: &V, f: impl Fn(f64, f64) -> std::result::Result<f64, String>) -> V {
    match (a.num(), b.num()) {
        (Ok(x), Ok(y)) => match f(x, y) {
            Ok(n) if n.is_finite() => V::Num(n),
            Ok(_) => V::Err("#NUM!".into()),
            Err(e) => V::Err(e),
        },
        (Err(e), _) | (_, Err(e)) => V::Err(e),
    }
}

fn compare(a: &V, b: &V, op: &str) -> V {
    use std::cmp::Ordering;
    let ord = match (a, b) {
        (V::Err(e), _) | (_, V::Err(e)) => return V::Err(e.clone()),
        (V::Str(x), V::Str(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (V::Str(_), _) => Ordering::Greater,
        (_, V::Str(_)) => Ordering::Less,
        _ => a.num().unwrap_or(0.0).partial_cmp(&b.num().unwrap_or(0.0)).unwrap_or(Ordering::Equal),
    };
    V::Bool(match op {
        "=" => ord == Ordering::Equal,
        "<>" => ord != Ordering::Equal,
        "<" => ord == Ordering::Less,
        ">" => ord == Ordering::Greater,
        "<=" => ord != Ordering::Greater,
        _ => ord != Ordering::Less,
    })
}

fn nums(args: &[V]) -> std::result::Result<Vec<f64>, String> {
    let mut out = Vec::new();
    for a in args {
        match a {
            V::Range(_) => {
                for v in a.flat() {
                    match v {
                        V::Num(n) => out.push(n),
                        V::Err(e) => return Err(e),
                        _ => {}
                    }
                }
            }
            v => out.push(v.num()?),
        }
    }
    Ok(out)
}

fn round_to(n: f64, digits: f64, mode: i32) -> f64 {
    let m = 10f64.powi(digits as i32);
    let x = n * m;
    let r = match mode {
        0 => (x.abs() + 1e-9).round() * x.signum(),
        1 => x.abs().ceil() * x.signum(),
        _ => x.abs().floor() * x.signum(),
    };
    r / m
}

/// Evaluates a criteria of SUMIF/COUNTIF (">10", "A社", "<>x").
fn matches(v: &V, crit: &V) -> bool {
    let c = crit.text().unwrap_or_default();
    for op in ["<=", ">=", "<>", "<", ">", "="] {
        if let Some(rest) = c.strip_prefix(op) {
            let rhs = match rest.parse::<f64>() {
                Ok(n) => V::Num(n),
                Err(_) => V::Str(rest.to_string()),
            };
            if let (V::Str(_), V::Num(_)) = (&rhs, v) {
                return op == "<>";
            }
            return matches!(compare(v, &rhs, op), V::Bool(true));
        }
    }
    match crit {
        V::Num(n) => v.num().map(|x| x == *n).unwrap_or(false),
        _ => v.text().map(|t| t.eq_ignore_ascii_case(&c)).unwrap_or(false),
    }
}

fn call(name: &str, args: Vec<V>) -> V {
    let r = (|| -> std::result::Result<V, String> {
        let arg = |i: usize| args.get(i).cloned().unwrap_or(V::Empty);
        Ok(match name {
            "SUM" => V::Num(nums(&args)?.iter().sum()),
            "AVERAGE" => {
                let n = nums(&args)?;
                if n.is_empty() {
                    return Err("#DIV/0!".into());
                }
                V::Num(n.iter().sum::<f64>() / n.len() as f64)
            }
            "MIN" => V::Num(nums(&args)?.into_iter().reduce(f64::min).unwrap_or(0.0)),
            "MAX" => V::Num(nums(&args)?.into_iter().reduce(f64::max).unwrap_or(0.0)),
            "COUNT" => V::Num(args.iter().flat_map(|a| a.flat()).filter(|v| matches!(v, V::Num(_))).count() as f64),
            "COUNTA" => V::Num(args.iter().flat_map(|a| a.flat()).filter(|v| !matches!(v, V::Empty)).count() as f64),
            "PRODUCT" => V::Num(nums(&args)?.iter().product()),
            "ROUND" => V::Num(round_to(arg(0).num()?, arg(1).num()?, 0)),
            "ROUNDUP" => V::Num(round_to(arg(0).num()?, arg(1).num()?, 1)),
            "ROUNDDOWN" | "TRUNC" => V::Num(round_to(arg(0).num()?, arg(1).num()?, 2)),
            "INT" => V::Num(arg(0).num()?.floor()),
            "ABS" => V::Num(arg(0).num()?.abs()),
            "MOD" => {
                let (a, b) = (arg(0).num()?, arg(1).num()?);
                if b == 0.0 {
                    return Err("#DIV/0!".into());
                }
                V::Num(a - b * (a / b).floor())
            }
            "SQRT" => V::Num(arg(0).num()?.sqrt()),
            "POWER" => V::Num(arg(0).num()?.powf(arg(1).num()?)),
            "IF" => {
                if arg(0).truthy()? {
                    if args.len() > 1 { arg(1) } else { V::Bool(true) }
                } else if args.len() > 2 {
                    arg(2)
                } else {
                    V::Bool(false)
                }
            }
            "IFERROR" => match arg(0) {
                V::Err(_) => arg(1),
                v => v,
            },
            "AND" => V::Bool(args.iter().flat_map(|a| a.flat()).filter(|v| !matches!(v, V::Empty)).map(|v| v.truthy()).collect::<std::result::Result<Vec<_>, _>>()?.iter().all(|b| *b)),
            "OR" => V::Bool(args.iter().flat_map(|a| a.flat()).filter(|v| !matches!(v, V::Empty)).map(|v| v.truthy()).collect::<std::result::Result<Vec<_>, _>>()?.iter().any(|b| *b)),
            "NOT" => V::Bool(!arg(0).truthy()?),
            "LEN" => V::Num(arg(0).text()?.chars().count() as f64),
            "UPPER" => V::Str(arg(0).text()?.to_uppercase()),
            "LOWER" => V::Str(arg(0).text()?.to_lowercase()),
            "TRIM" => V::Str(arg(0).text()?.split_whitespace().collect::<Vec<_>>().join(" ")),
            "LEFT" => {
                let n = if args.len() > 1 { arg(1).num()? as usize } else { 1 };
                V::Str(arg(0).text()?.chars().take(n).collect())
            }
            "RIGHT" => {
                let n = if args.len() > 1 { arg(1).num()? as usize } else { 1 };
                let s: Vec<char> = arg(0).text()?.chars().collect();
                V::Str(s[s.len().saturating_sub(n)..].iter().collect())
            }
            "MID" => {
                let s: Vec<char> = arg(0).text()?.chars().collect();
                let st = (arg(1).num()? as usize).max(1) - 1;
                let n = arg(2).num()? as usize;
                V::Str(s.iter().skip(st).take(n).collect())
            }
            "CONCAT" | "CONCATENATE" => {
                let mut s = String::new();
                for a in &args {
                    for v in a.flat() {
                        s.push_str(&v.text()?);
                    }
                }
                V::Str(s)
            }
            "SUMIF" | "COUNTIF" | "AVERAGEIF" => {
                let range = arg(0).flat();
                let crit = arg(1);
                let sum_range = if args.len() > 2 { arg(2).flat() } else { range.clone() };
                let mut total = 0.0;
                let mut count = 0.0;
                for (i, v) in range.iter().enumerate() {
                    if matches(v, &crit) {
                        count += 1.0;
                        if let Some(V::Num(n)) = sum_range.get(i) {
                            total += n;
                        }
                    }
                }
                match name {
                    "COUNTIF" => V::Num(count),
                    "SUMIF" => V::Num(total),
                    _ if count == 0.0 => return Err("#DIV/0!".into()),
                    _ => V::Num(total / count),
                }
            }
            _ => return Err("#NAME?".into()),
        })
    })();
    r.unwrap_or_else(V::Err)
}

/// The value of the formula in the cell, as text.
pub fn calc_cell(b: &Book, sheet: &str, r: &str) -> Result<String> {
    let (col, row) = parse_ref(r)?;
    let ws = b.sheet(sheet)?;
    let Some(c) = ws.cell((col, row)) else { return Ok(String::new()) };
    let v = if c.is_formula() {
        let mut ev = Ev { b, sheet: sheet.to_string(), toks: tokenize(c.formula())?, pos: 0, depth: 0 };
        let v = ev.expr()?;
        if ev.pos < ev.toks.len() {
            return Err(Error::msg(format!("formula: unexpected {:?}", ev.toks[ev.pos])));
        }
        v
    } else {
        let mut ev = Ev { b, sheet: sheet.to_string(), toks: Vec::new(), pos: 0, depth: 0 };
        ev.cell(sheet, col, row)?
    };
    Ok(match v {
        V::Num(n) => fmt_calc(n),
        V::Err(e) => e,
        V::Range(_) => "#VALUE!".into(),
        other => other.text().unwrap_or_default(),
    })
}

/// A computed number as text (no binary noise: 15 significant digits).
fn fmt_calc(n: f64) -> String {
    let s = format!("{:.15e}", n);
    let v: f64 = s.parse().unwrap_or(n);
    let r: f64 = format!("{v:.15}").parse().unwrap_or(v);
    fmt_num(super::numfmt::general(r).parse().unwrap_or(r))
}
