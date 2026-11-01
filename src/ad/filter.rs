//! LDAP search filters (RFC 4515): escaping values, checking the filters
//! scripts pass, and (for the fake directory of the tests) evaluating them.

use crate::{Result, err};

/// The bitwise AND and OR matching rules and the "in chain" rule of Active
/// Directory.
pub const RULE_AND: &str = "1.2.840.113556.1.4.803";
pub const RULE_OR: &str = "1.2.840.113556.1.4.804";
pub const RULE_CHAIN: &str = "1.2.840.113556.1.4.1941";

/// Escapes a value for a filter. With wildcards, "*" is left as is (a
/// substring match).
pub fn escape(v: &str, wildcards: bool) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '*' if wildcards => out.push('*'),
            '*' | '(' | ')' | '\\' | '\0' => out.push_str(&format!("\\{:02x}", c as u32)),
            _ => out.push(c),
        }
    }
    out
}

/// Escapes bytes (a binary value such as a SID) for a filter.
pub fn escape_bytes(b: &[u8]) -> String {
    b.iter().map(|x| format!("\\{x:02x}")).collect()
}

/// The filters joined with "&" (the only one as is).
pub fn and(parts: &[String]) -> String {
    join('&', parts)
}

/// The filters joined with "|".
pub fn or(parts: &[String]) -> String {
    join('|', parts)
}

fn join(op: char, parts: &[String]) -> String {
    match parts {
        [] => "(objectClass=*)".into(),
        [one] => one.clone(),
        _ => format!("({op}{})", parts.concat()),
    }
}

/// A parsed filter.
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    And(Vec<Filter>),
    Or(Vec<Filter>),
    Not(Box<Filter>),
    Present(String),
    /// attr=value; with wildcards the value parts between the "*"s.
    Equal(String, Vec<u8>),
    Substr(String, Vec<Vec<u8>>),
    Ge(String, Vec<u8>),
    Le(String, Vec<u8>),
    Approx(String, Vec<u8>),
    /// attr:rule:=value
    Ext(String, String, Vec<u8>),
}

/// Parses a filter; a filter without the outer parentheses ("cn=a*") is
/// accepted too.
pub fn parse(s: &str) -> Result<Filter> {
    let t = s.trim();
    let src = if t.starts_with('(') { t.to_string() } else { format!("({t})") };
    let mut p = Parser { s: src.as_bytes(), i: 0 };
    let f = p.filter().ok_or_else(|| err!("ad.badFilter", s))?;
    if p.i != p.s.len() {
        return Err(err!("ad.badFilter", s));
    }
    Ok(f)
}

/// The filter as a string with its outer parentheses ("cn=a" -> "(cn=a)"),
/// checked.
pub fn normalize(s: &str) -> Result<String> {
    parse(s)?;
    let t = s.trim();
    Ok(if t.starts_with('(') { t.to_string() } else { format!("({t})") })
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn skip_space(&mut self) {
        while self.peek() == Some(b' ') {
            self.i += 1;
        }
    }

    fn filter(&mut self) -> Option<Filter> {
        self.skip_space();
        if !self.eat(b'(') {
            return None;
        }
        self.skip_space();
        let f = match self.peek()? {
            b'&' => {
                self.i += 1;
                Filter::And(self.list()?)
            }
            b'|' => {
                self.i += 1;
                Filter::Or(self.list()?)
            }
            b'!' => {
                self.i += 1;
                Filter::Not(Box::new(self.filter()?))
            }
            _ => self.item()?,
        };
        self.skip_space();
        if !self.eat(b')') {
            return None;
        }
        Some(f)
    }

    fn list(&mut self) -> Option<Vec<Filter>> {
        let mut v = Vec::new();
        loop {
            self.skip_space();
            if self.peek() != Some(b'(') {
                break;
            }
            v.push(self.filter()?);
        }
        if v.is_empty() { None } else { Some(v) }
    }

    fn attr(&mut self) -> String {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'-' || c == b'.' || c == b';' || c == b'_' {
                self.i += 1;
            } else {
                break;
            }
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }

    fn item(&mut self) -> Option<Filter> {
        let attr = self.attr();
        if self.eat(b':') {
            // attr:rule:=value (the "dn" flag is not supported)
            let rule = self.attr();
            if attr.is_empty() || rule.is_empty() || !self.eat(b':') || !self.eat(b'=') {
                return None;
            }
            let v = self.value(false)?;
            return Some(Filter::Ext(attr, rule, v.concat()));
        }
        if attr.is_empty() {
            return None;
        }
        let op = match (self.peek()?, self.s.get(self.i + 1).copied()) {
            (b'>', Some(b'=')) => b'>',
            (b'<', Some(b'=')) => b'<',
            (b'~', Some(b'=')) => b'~',
            (b'=', _) => b'=',
            _ => return None,
        };
        self.i += if op == b'=' { 1 } else { 2 };
        let parts = self.value(op == b'=')?;
        Some(match op {
            b'>' => Filter::Ge(attr, parts.concat()),
            b'<' => Filter::Le(attr, parts.concat()),
            b'~' => Filter::Approx(attr, parts.concat()),
            _ if parts.len() == 2 && parts.iter().all(|p| p.is_empty()) => Filter::Present(attr),
            _ if parts.len() == 1 => Filter::Equal(attr, parts.into_iter().next().unwrap()),
            _ => Filter::Substr(attr, parts),
        })
    }

    /// The value up to ")", split at the "*"s when wildcards is set.
    fn value(&mut self, wildcards: bool) -> Option<Vec<Vec<u8>>> {
        let mut parts = vec![Vec::new()];
        loop {
            match self.peek()? {
                b')' => break,
                b'(' => return None,
                b'*' if wildcards => {
                    parts.push(Vec::new());
                    self.i += 1;
                }
                b'\\' => {
                    let h = self.s.get(self.i + 1..self.i + 3)?;
                    let h = std::str::from_utf8(h).ok()?;
                    parts.last_mut().unwrap().push(u8::from_str_radix(h, 16).ok()?);
                    self.i += 3;
                }
                c => {
                    parts.last_mut().unwrap().push(c);
                    self.i += 1;
                }
            }
        }
        if !wildcards && parts[0].is_empty() {
            return None;
        }
        Some(parts)
    }
}

/// Evaluates filters against entries (the fake directory of the tests).
#[cfg(test)]
pub mod eval {
    use super::*;

    /// What a filter needs of an entry: its values of an attribute, and
    /// whether it is a member of a group, directly or through other groups.
    pub trait Subject {
        fn values(&self, attr: &str) -> Vec<Vec<u8>>;
        fn in_chain(&self, attr: &str, dn: &[u8]) -> bool;
    }

    fn lower(b: &[u8]) -> String {
        String::from_utf8_lossy(b).to_lowercase()
    }

    fn num(b: &[u8]) -> Option<i64> {
        std::str::from_utf8(b).ok()?.parse().ok()
    }

    pub fn matches(f: &Filter, e: &dyn Subject) -> bool {
        match f {
            Filter::And(v) => v.iter().all(|f| matches(f, e)),
            Filter::Or(v) => v.iter().any(|f| matches(f, e)),
            Filter::Not(f) => !matches(f, e),
            Filter::Present(a) => a == "*" || !e.values(a).is_empty() || a.eq_ignore_ascii_case("objectClass"),
            Filter::Equal(a, v) | Filter::Approx(a, v) => e.values(a).iter().any(|x| x == v || lower(x) == lower(v)),
            Filter::Substr(a, parts) => e.values(a).iter().any(|x| substr(&lower(x), parts)),
            Filter::Ge(a, v) => e.values(a).iter().any(|x| match (num(x), num(v)) {
                (Some(x), Some(v)) => x >= v,
                _ => lower(x) >= lower(v),
            }),
            Filter::Le(a, v) => e.values(a).iter().any(|x| match (num(x), num(v)) {
                (Some(x), Some(v)) => x <= v,
                _ => lower(x) <= lower(v),
            }),
            Filter::Ext(a, rule, v) => match rule.as_str() {
                RULE_AND | RULE_OR => {
                    let Some(mask) = num(v) else { return false };
                    e.values(a).iter().filter_map(|x| num(x)).any(|x| if rule == RULE_AND { x & mask == mask } else { x & mask != 0 })
                }
                RULE_CHAIN => e.in_chain(a, v),
                _ => false,
            },
        }
    }

    fn substr(x: &str, parts: &[Vec<u8>]) -> bool {
        let parts: Vec<String> = parts.iter().map(|p| lower(p)).collect();
        let (first, last) = (&parts[0], &parts[parts.len() - 1]);
        if !x.starts_with(first.as_str()) || x.len() < first.len() + last.len() || !x.ends_with(last.as_str()) {
            return false;
        }
        let mut rest = &x[first.len()..x.len() - last.len()];
        for p in &parts[1..parts.len() - 1] {
            match rest.find(p.as_str()) {
                Some(i) => rest = &rest[i + p.len()..],
                None => return false,
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes() {
        assert_eq!(escape("a*(b)\\", true), "a*\\28b\\29\\5c");
        assert_eq!(escape("a*", false), "a\\2a");
        assert_eq!(escape_bytes(&[1, 0xab]), "\\01\\ab");
        assert_eq!(and(&["(a=1)".into()]), "(a=1)");
        assert_eq!(or(&["(a=1)".into(), "(b=2)".into()]), "(|(a=1)(b=2))");
    }

    #[test]
    fn parses() {
        let _g = crate::test_lang("ja");
        assert_eq!(parse("cn=a").unwrap(), Filter::Equal("cn".into(), b"a".to_vec()));
        assert_eq!(parse("(cn=*)").unwrap(), Filter::Present("cn".into()));
        assert_eq!(parse("(cn=山*田)").unwrap(), Filter::Substr("cn".into(), vec!["山".as_bytes().to_vec(), "田".as_bytes().to_vec()]));
        assert_eq!(parse("(cn=a\\2a)").unwrap(), Filter::Equal("cn".into(), b"a*".to_vec()));
        assert_eq!(
            parse("(&(objectCategory=person)(!(userAccountControl:1.2.840.113556.1.4.803:=2))(lockoutTime>=1))").unwrap(),
            Filter::And(vec![
                Filter::Equal("objectCategory".into(), b"person".to_vec()),
                Filter::Not(Box::new(Filter::Ext("userAccountControl".into(), RULE_AND.into(), b"2".to_vec()))),
                Filter::Ge("lockoutTime".into(), b"1".to_vec()),
            ])
        );
        for bad in ["", "(cn=a", "(cn=a))", "(&)", "(=a)", "(cn~a)", "cn=(a)", "(cn=\\zz)"] {
            let e = parse(bad).unwrap_err().to_string();
            assert!(e.contains("検索条件"), "{bad}: {e}");
        }
        assert_eq!(normalize(" mail=*@example.com ").unwrap(), "(mail=*@example.com)");
    }
}
