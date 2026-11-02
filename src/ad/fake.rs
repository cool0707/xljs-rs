//! A directory for the tests: the entries of testdata/ad/directory.json,
//! searched with the filter evaluator. Values are strings; "sid:S-1-...",
//! "guid:..." and "hex:..." stand for binary values. The server "down"
//! cannot be reached.

use std::sync::LazyLock;

use serde_json::Value as J;

use super::filter::eval::{Subject, matches};
use super::{Directory, Entry, Query, Scope, filter, value};
use crate::{Result, err};

struct Data {
    root: Entry,
    user: String,
    computer: String,
    entries: Vec<Entry>,
}

fn bytes(s: &str) -> Vec<u8> {
    if let Some(x) = s.strip_prefix("sid:") {
        return value::sid_bytes(x).expect("bad sid in the fake directory");
    }
    if let Some(x) = s.strip_prefix("guid:") {
        return value::guid_bytes(x).expect("bad guid in the fake directory");
    }
    if let Some(x) = s.strip_prefix("hex:") {
        return (0..x.len() / 2).map(|i| u8::from_str_radix(&x[i * 2..i * 2 + 2], 16).unwrap()).collect();
    }
    s.as_bytes().to_vec()
}

fn entry(dn: &str, attrs: &J) -> Entry {
    let attrs = attrs
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            let vals = match v {
                J::Array(a) => a.iter().map(|x| bytes(x.as_str().unwrap())).collect(),
                J::String(s) => vec![bytes(s)],
                _ => panic!("bad value of {k} in the fake directory"),
            };
            (k.clone(), vals)
        })
        .collect();
    Entry { dn: dn.into(), attrs }
}

static DATA: LazyLock<Data> = LazyLock::new(|| {
    let src = std::fs::read_to_string(crate::testutil::repo_path("testdata/ad/directory.json")).unwrap();
    let j: J = serde_json::from_str(&src).unwrap();
    Data {
        root: entry("", &j["rootDSE"]),
        user: j["self"]["user"].as_str().unwrap().into(),
        computer: j["self"]["computer"].as_str().unwrap().into(),
        entries: j["entries"].as_array().unwrap().iter().map(|e| entry(e["dn"].as_str().unwrap(), &e["attrs"])).collect(),
    }
});

fn values(e: &Entry, attr: &str) -> Vec<Vec<u8>> {
    if attr.eq_ignore_ascii_case("distinguishedName") {
        return vec![e.dn.clone().into_bytes()];
    }
    e.attrs.iter().filter(|(k, _)| k.eq_ignore_ascii_case(attr)).flat_map(|(_, v)| v.clone()).collect()
}

fn find(dn: &str) -> Option<&'static Entry> {
    DATA.entries.iter().find(|e| e.dn.eq_ignore_ascii_case(dn))
}

struct Subj<'a>(&'a Entry);

impl Subject for Subj<'_> {
    fn values(&self, attr: &str) -> Vec<Vec<u8>> {
        values(self.0, attr)
    }
    fn in_chain(&self, attr: &str, dn: &[u8]) -> bool {
        let target = String::from_utf8_lossy(dn).to_lowercase();
        let mut seen = Vec::new();
        let mut todo: Vec<String> = values(self.0, attr).iter().map(|v| String::from_utf8_lossy(v).into_owned()).collect();
        while let Some(g) = todo.pop() {
            if g.to_lowercase() == target {
                return true;
            }
            if seen.contains(&g) {
                continue;
            }
            if let Some(e) = find(&g) {
                todo.extend(values(e, attr).iter().map(|v| String::from_utf8_lossy(v).into_owned()));
            }
            seen.push(g);
        }
        false
    }
}

fn under(dn: &str, base: &str) -> bool {
    let (dn, base) = (dn.to_lowercase(), base.to_lowercase());
    dn == base || dn.ends_with(&format!(",{base}"))
}

fn parent(dn: &str) -> &str {
    dn.split_once(',').map(|(_, p)| p).unwrap_or("")
}

/// Only the asked attributes ("*" for all; constructed ones only by name).
fn select(e: &Entry, attrs: &[String]) -> Entry {
    let all = attrs.iter().any(|a| a == "*");
    let attrs = e
        .attrs
        .iter()
        .filter(|(k, _)| attrs.iter().any(|a| a.eq_ignore_ascii_case(k)) || (all && !k.starts_with("msDS-User-Account-Control-Computed")))
        .cloned()
        .collect();
    Entry { dn: e.dn.clone(), attrs }
}

pub struct Fake;

pub fn open(server: &str) -> Result<Box<dyn Directory>> {
    if server == "down" {
        return Err(super::server_down(server));
    }
    Ok(Box::new(Fake))
}

pub fn self_dn(computer: bool) -> Result<String> {
    Ok(if computer { DATA.computer.clone() } else { DATA.user.clone() })
}

impl Directory for Fake {
    fn search(&self, q: &Query, each: &mut dyn FnMut(Entry) -> bool) -> Result<()> {
        let f = filter::parse(&q.filter)?;
        if q.base.is_empty() && q.scope == Scope::Base {
            each(select(&DATA.root, &q.attrs));
            return Ok(());
        }
        if find(&q.base).is_none() && !DATA.entries.iter().any(|e| under(&e.dn, &q.base)) {
            return Err(err!("ad.noSuchObject", q.base));
        }
        let mut n = 0;
        for e in &DATA.entries {
            let inside = match q.scope {
                Scope::Base => e.dn.eq_ignore_ascii_case(&q.base),
                Scope::OneLevel => parent(&e.dn).eq_ignore_ascii_case(&q.base),
                Scope::Subtree => under(&e.dn, &q.base),
            };
            if !inside || !matches(&f, &Subj(e)) {
                continue;
            }
            if !each(select(e, &q.attrs)) {
                break;
            }
            n += 1;
            if q.limit > 0 && n >= q.limit {
                break;
            }
        }
        Ok(())
    }
}
