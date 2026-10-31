//! Active Directory: searching the domain the computer belongs to (users,
//! computers, groups) and reading the signed-in user and this computer,
//! with the credentials of the signed-in user. One search covers what
//! PowerShell splits into Get-ADUser / Get-ADComputer / Get-ADGroup and
//! Search-ADAccount (disabled, locked out, expired and inactive accounts).
//!
//! The directory itself is behind [`Directory`]: wldap32 on Windows, the
//! entries of testdata/ad in the tests.

pub mod filter;
pub mod value;

#[cfg(test)]
mod fake;
#[cfg(all(windows, not(test)))]
mod ldap;
#[cfg(all(windows, test))]
#[allow(dead_code)]
mod ldap;

use crate::{Result, err};
pub use value::Val;

/// An entry as the directory returns it.
#[derive(Debug, Clone)]
pub struct Entry {
    pub dn: String,
    pub attrs: Vec<(String, Vec<Vec<u8>>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    Base,
    OneLevel,
    #[default]
    Subtree,
}

/// A search: limit 0 means no limit.
pub struct Query {
    pub base: String,
    pub scope: Scope,
    pub filter: String,
    pub attrs: Vec<String>,
    pub limit: usize,
}

pub trait Directory {
    /// Calls each for the entries found until it returns false.
    fn search(&self, q: &Query, each: &mut dyn FnMut(Entry) -> bool) -> Result<()>;
}

fn open_directory(server: &str) -> Result<Box<dyn Directory>> {
    #[cfg(test)]
    return fake::open(server);
    #[cfg(all(windows, not(test)))]
    return Ok(Box::new(ldap::Ldap::connect(server)?));
    #[cfg(all(not(windows), not(test)))]
    {
        let _ = server;
        Err(err!("ad.unsupported"))
    }
}

/// The error for a domain controller that cannot be reached.
fn server_down(server: &str) -> crate::Error {
    if server.is_empty() { err!("ad.serverDown") } else { err!("ad.serverDownNamed", server) }
}

/// Whether the computer is joined to a domain (without asking the domain).
pub fn joined() -> bool {
    #[cfg(test)]
    return true;
    #[cfg(all(windows, not(test)))]
    return ldap::joined();
    #[cfg(all(not(windows), not(test)))]
    false
}

fn self_dn(computer: bool) -> Result<String> {
    #[cfg(test)]
    return fake::self_dn(computer);
    #[cfg(all(windows, not(test)))]
    return ldap::self_dn(computer);
    #[cfg(all(not(windows), not(test)))]
    {
        let _ = computer;
        Err(err!("ad.unsupported"))
    }
}

/// An object for the script: its properties in order.
pub type Obj = Vec<(String, Val)>;

/// The attributes read by default (more with properties).
pub const DEFAULT_ATTRS: &[&str] = &[
    "name", "distinguishedName", "objectClass", "objectGUID", "objectSid", "sAMAccountName", "userPrincipalName", "displayName",
    "givenName", "sn", "mail", "department", "title", "company", "description", "employeeID", "telephoneNumber", "mobile",
    "physicalDeliveryOfficeName", "manager", "memberOf", "userAccountControl", "lockoutTime", "lastLogonTimestamp", "pwdLastSet",
    "accountExpires", "whenCreated", "whenChanged", "dNSHostName", "operatingSystem", "operatingSystemVersion", "groupType",
    "managedBy",
];

/// Constructed attribute with the locked-out and password-expired flags.
const COMPUTED_UAC: &str = "msDS-User-Account-Control-Computed";

const UF_ACCOUNTDISABLE: i64 = 0x2;
const UF_LOCKOUT: i64 = 0x10;
const UF_DONT_EXPIRE_PASSWD: i64 = 0x10000;
const UF_PASSWORD_EXPIRED: i64 = 0x800000;

/// A condition on one attribute (the filter object of search options).
#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    /// Equal; "*" matches any characters.
    Eq(String),
    Present(bool),
    Any(Vec<Cond>),
}

/// The options of a search. Option<bool> conditions: None = either way.
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// "user" (the default), "computer", "group", "contact",
    /// "organizationalUnit" ("ou") or "all"; several = any of them.
    pub types: Vec<String>,
    /// name, sAMAccountName, displayName, userPrincipalName or mail.
    pub name: Option<String>,
    pub conds: Vec<(String, Cond)>,
    /// LDAP filters.
    pub filters: Vec<String>,
    pub disabled: Option<bool>,
    pub locked_out: Option<bool>,
    pub expired: Option<bool>,
    pub expiring_days: Option<f64>,
    pub inactive_days: Option<f64>,
    pub password_expired: Option<bool>,
    pub password_never_expires: Option<bool>,
    pub member_of: Option<String>,
    pub recursive: bool,
    /// "" for the whole domain.
    pub base: String,
    pub scope: Scope,
    pub properties: Vec<String>,
    pub limit: usize,
}

fn type_filter(t: &str) -> Result<Option<&'static str>> {
    Ok(Some(match t.to_ascii_lowercase().as_str() {
        "user" => "(&(objectCategory=person)(objectClass=user))",
        "computer" => "(objectCategory=computer)",
        "group" => "(objectCategory=group)",
        "contact" => "(&(objectCategory=person)(objectClass=contact))",
        "organizationalunit" | "ou" => "(objectCategory=organizationalUnit)",
        "all" | "*" => return Ok(None),
        _ => return Err(err!("ad.badType", t)),
    }))
}

/// The filter for the types (None: any object).
fn types_filter(types: &[String]) -> Result<Option<String>> {
    let mut parts = Vec::new();
    for t in types {
        match type_filter(t)? {
            Some(f) => parts.push(f.to_string()),
            None => return Ok(None),
        }
    }
    Ok(if parts.is_empty() { None } else { Some(filter::or(&parts)) })
}

fn cond_filter(attr: &str, c: &Cond) -> Result<String> {
    if attr.is_empty() || !attr.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b';') {
        return Err(err!("ad.badAttribute", attr));
    }
    Ok(match c {
        Cond::Eq(v) => format!("({attr}={})", filter::escape(v, true)),
        Cond::Present(true) => format!("({attr}=*)"),
        Cond::Present(false) => format!("(!({attr}=*))"),
        Cond::Any(v) => {
            let parts = v.iter().map(|c| cond_filter(attr, c)).collect::<Result<Vec<_>>>()?;
            if parts.is_empty() { "(!(objectClass=*))".into() } else { filter::or(&parts) }
        }
    })
}

fn bit(attr: &str, mask: i64, on: bool) -> String {
    let f = format!("({attr}:{}:={mask})", filter::RULE_AND);
    if on { f } else { format!("(!{f})") }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

const DAY_MS: f64 = 86_400_000.0;

/// "DC=corp,DC=example,DC=com" -> "corp.example.com".
pub fn dns_name(dn: &str) -> String {
    dn.split(',')
        .filter_map(|p| {
            let (k, v) = p.trim().split_once('=')?;
            k.eq_ignore_ascii_case("DC").then(|| v.to_string())
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Whether s looks like a distinguished name ("CN=...,DC=...").
pub fn is_dn(s: &str) -> bool {
    match s.split_once('=') {
        Some((k, _)) => !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && s.contains(','),
        None => false,
    }
}

struct RootDse {
    default_nc: String,
    config_nc: String,
    root_nc: String,
    server: String,
}

/// A connection to a domain controller.
pub struct Session {
    dir: Box<dyn Directory>,
    root: RootDse,
}

fn first(e: &Entry, attr: &str) -> String {
    e.attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(attr))
        .and_then(|(_, v)| v.first())
        .map(|v| String::from_utf8_lossy(v).into_owned())
        .unwrap_or_default()
}

fn num_of(o: &Obj, key: &str) -> Option<i64> {
    o.iter().find(|(k, _)| k == key).and_then(|(_, v)| if let Val::Num(n) = v { Some(*n as i64) } else { None })
}

/// The attribute names to ask for and the order of the properties: the
/// defaults, then the asked ones (as the script spells them).
fn attr_list(props: &[String]) -> Vec<String> {
    let mut out: Vec<String> = DEFAULT_ATTRS.iter().map(|s| s.to_string()).collect();
    for p in props {
        let p = p.trim();
        if !p.is_empty() && !out.iter().any(|a| a.eq_ignore_ascii_case(p)) {
            out.push(p.to_string());
        }
    }
    out
}

impl Session {
    pub fn open(server: &str) -> Result<Session> {
        let dir = open_directory(server)?;
        let mut root = None;
        let q = Query {
            base: String::new(),
            scope: Scope::Base,
            filter: "(objectClass=*)".into(),
            attrs: ["defaultNamingContext", "configurationNamingContext", "rootDomainNamingContext", "dnsHostName"].map(String::from).to_vec(),
            limit: 0,
        };
        dir.search(&q, &mut |e| {
            root = Some(RootDse {
                default_nc: first(&e, "defaultNamingContext"),
                config_nc: first(&e, "configurationNamingContext"),
                root_nc: first(&e, "rootDomainNamingContext"),
                server: first(&e, "dnsHostName"),
            });
            false
        })?;
        let root = root.filter(|r| !r.default_nc.is_empty()).ok_or_else(|| server_down(server))?;
        Ok(Session { dir, root })
    }

    /// { name, netbiosName, distinguishedName, forest, server }
    pub fn domain(&self) -> Result<Obj> {
        let mut netbios = Val::Null;
        if !self.root.config_nc.is_empty() {
            let q = Query {
                base: format!("CN=Partitions,{}", self.root.config_nc),
                scope: Scope::OneLevel,
                filter: format!("(&(objectClass=crossRef)(nCName={}))", filter::escape(&self.root.default_nc, false)),
                attrs: vec!["nETBIOSName".into()],
                limit: 1,
            };
            // Not every account may read the configuration partition.
            let _ = self.dir.search(&q, &mut |e| {
                let n = first(&e, "nETBIOSName");
                if !n.is_empty() {
                    netbios = Val::Str(n);
                }
                false
            });
        }
        Ok(vec![
            ("name".into(), Val::Str(dns_name(&self.root.default_nc))),
            ("netbiosName".into(), netbios),
            ("distinguishedName".into(), Val::Str(self.root.default_nc.clone())),
            ("forest".into(), Val::Str(dns_name(&self.root.root_nc))),
            ("server".into(), Val::Str(self.root.server.clone())),
        ])
    }

    /// Runs a query, reading the rest of the values that came in ranges
    /// ("member;range=0-1499").
    fn query(&self, q: &Query, each: &mut dyn FnMut(Entry) -> bool) -> Result<()> {
        let mut err = None;
        self.dir.search(q, &mut |mut e| {
            if let Err(x) = self.complete_ranges(&mut e) {
                err = Some(x);
                return false;
            }
            each(e)
        })?;
        err.map_or(Ok(()), Err)
    }

    fn complete_ranges(&self, e: &mut Entry) -> Result<()> {
        for (name, vals) in e.attrs.iter_mut() {
            let Some((attr, range)) = name.split_once(";range=") else { continue };
            let attr = attr.to_string();
            let mut range = range.to_string();
            while let Some((_, hi)) = range.split_once('-') {
                let Ok(hi) = hi.parse::<u64>() else { break }; // "*": the last range
                let ask = format!("{attr};range={}-*", hi + 1);
                let q = Query { base: e.dn.clone(), scope: Scope::Base, filter: "(objectClass=*)".into(), attrs: vec![ask], limit: 0 };
                let mut next = None;
                self.dir.search(&q, &mut |r| {
                    next = r.attrs.into_iter().find(|(k, _)| k.to_lowercase().starts_with(&format!("{};range=", attr.to_lowercase())));
                    false
                })?;
                let Some((k, more)) = next else { break };
                vals.extend(more);
                range = k.split_once(";range=").map(|(_, r)| r.to_string()).unwrap_or_default();
            }
            *name = attr;
        }
        Ok(())
    }

    /// The query attributes for the properties.
    fn ask(&self, props: &[String]) -> (Vec<String>, Vec<String>) {
        let order = attr_list(props);
        let mut ask = order.clone();
        if props.iter().any(|p| p.trim() == "*") {
            ask.retain(|a| a != "*");
            ask.insert(0, "*".into());
        }
        if !ask.iter().any(|a| a.eq_ignore_ascii_case(COMPUTED_UAC)) {
            ask.push(COMPUTED_UAC.into());
        }
        (ask, order)
    }

    /// An entry as an object: type, name, distinguishedName, the properties
    /// in order, then the account and group states.
    fn object(e: Entry, order: &[String]) -> Obj {
        let all = order.iter().any(|a| a == "*");
        let keep_computed = order.iter().any(|a| a.eq_ignore_ascii_case(COMPUTED_UAC));
        let mut attrs: Vec<Option<(String, Vec<Vec<u8>>)>> = e.attrs.into_iter().map(Some).collect();
        let mut take = |name: &str| -> Option<(String, Vec<Vec<u8>>)> {
            attrs.iter_mut().find(|a| a.as_ref().is_some_and(|(k, _)| k.eq_ignore_ascii_case(name))).and_then(Option::take)
        };
        let classes = take("objectClass").map(|(_, v)| v).unwrap_or_default();
        let classes: Vec<String> = classes.iter().map(|c| String::from_utf8_lossy(c).into_owned()).collect();
        let has = |c: &str| classes.iter().any(|x| x.eq_ignore_ascii_case(c));
        let kind = if has("computer") {
            "computer".to_string()
        } else if has("user") {
            "user".into()
        } else if has("group") {
            "group".into()
        } else if has("contact") {
            "contact".into()
        } else if has("organizationalUnit") {
            "organizationalUnit".into()
        } else {
            classes.last().cloned().unwrap_or_default()
        };
        let mut o: Obj = vec![("type".into(), Val::Str(kind))];
        let name = take("name").and_then(|(_, v)| v.into_iter().next()).map(|v| String::from_utf8_lossy(&v).into_owned());
        let name = name.unwrap_or_else(|| {
            let rdn = e.dn.split(',').next().unwrap_or("");
            rdn.split_once('=').map(|(_, v)| v.to_string()).unwrap_or_default()
        });
        o.push(("name".into(), Val::Str(name)));
        take("distinguishedName");
        o.push(("distinguishedName".into(), Val::Str(e.dn.clone())));
        let computed = take(COMPUTED_UAC);
        for key in order {
            match key.as_str() {
                "*" | "name" | "distinguishedName" => continue,
                "objectClass" => o.push((key.clone(), Val::List(classes.iter().cloned().map(Val::Str).collect()))),
                _ => {
                    if key.eq_ignore_ascii_case(COMPUTED_UAC) {
                        continue;
                    }
                    match take(key) {
                        Some((_, v)) => o.push((key.clone(), value::convert(key, &v))),
                        None if value::is_in(value::MULTI, key) => o.push((key.clone(), Val::List(Vec::new()))),
                        None => {}
                    }
                }
            }
        }
        if all {
            for (k, v) in attrs.into_iter().flatten() {
                if !k.eq_ignore_ascii_case(COMPUTED_UAC) {
                    let val = value::convert(&k, &v);
                    o.push((k, val));
                }
            }
        }
        let computed = computed.and_then(|(_, v)| v.first().and_then(|b| String::from_utf8_lossy(b).trim().parse::<i64>().ok()));
        if keep_computed && let Some(c) = computed {
            o.push((COMPUTED_UAC.into(), Val::Num(c as f64)));
        }
        if let Some(uac) = num_of(&o, "userAccountControl") {
            o.push(("enabled".into(), Val::Bool(uac & UF_ACCOUNTDISABLE == 0)));
            let c = computed.unwrap_or(0);
            o.push(("lockedOut".into(), Val::Bool(c & UF_LOCKOUT != 0)));
            o.push(("passwordExpired".into(), Val::Bool(c & UF_PASSWORD_EXPIRED != 0)));
            o.push(("passwordNeverExpires".into(), Val::Bool(uac & UF_DONT_EXPIRE_PASSWD != 0)));
        }
        if let Some(gt) = num_of(&o, "groupType") {
            let scope = if gt & 0x2 != 0 {
                "global"
            } else if gt & 0x4 != 0 {
                "domainLocal"
            } else if gt & 0x8 != 0 {
                "universal"
            } else {
                "builtin"
            };
            o.push(("groupScope".into(), Val::Str(scope.into())));
            let cat = if gt & 0x8000_0000 != 0 { "security" } else { "distribution" };
            o.push(("groupCategory".into(), Val::Str(cat.into())));
        }
        o
    }

    fn base(&self, base: &str) -> String {
        if base.is_empty() { self.root.default_nc.clone() } else { base.to_string() }
    }

    /// The objects the options select.
    pub fn search(&self, o: &SearchOptions) -> Result<Vec<Obj>> {
        let mut parts = Vec::new();
        let types = if o.types.is_empty() { vec!["user".to_string()] } else { o.types.clone() };
        if let Some(f) = types_filter(&types)? {
            parts.push(f);
        }
        if let Some(n) = &o.name {
            let v = filter::escape(n, true);
            let alts: Vec<String> =
                ["name", "sAMAccountName", "displayName", "userPrincipalName", "mail"].iter().map(|a| format!("({a}={v})")).collect();
            parts.push(filter::or(&alts));
        }
        for (k, c) in &o.conds {
            parts.push(cond_filter(k, c)?);
        }
        for f in &o.filters {
            parts.push(filter::normalize(f)?);
        }
        if let Some(d) = o.disabled {
            parts.push(bit("userAccountControl", UF_ACCOUNTDISABLE, d));
        }
        if let Some(p) = o.password_never_expires {
            parts.push(bit("userAccountControl", UF_DONT_EXPIRE_PASSWD, p));
        }
        let now = value::to_filetime(now_ms());
        if o.locked_out == Some(true) {
            parts.push("(lockoutTime>=1)".into());
        }
        if let Some(x) = o.expired {
            let f = format!("(&(accountExpires>=1)(accountExpires<={now}))");
            parts.push(if x { f } else { format!("(!{f})") });
        }
        if let Some(d) = o.expiring_days {
            let until = value::to_filetime(now_ms() + (d * DAY_MS) as i64);
            parts.push(format!("(&(accountExpires>={now})(accountExpires<={until}))"));
        }
        if let Some(d) = o.inactive_days {
            let since = value::to_filetime(now_ms() - (d * DAY_MS) as i64);
            parts.push(format!("(|(!(lastLogonTimestamp=*))(lastLogonTimestamp<={since}))"));
        }
        if let Some(g) = &o.member_of {
            let dn = if is_dn(g) {
                g.clone()
            } else {
                match self.get(g, &["group".to_string()], &[])? {
                    Some(obj) => obj.iter().find(|(k, _)| k == "distinguishedName").map(|(_, v)| if let Val::Str(s) = v { s.clone() } else { String::new() }).unwrap_or_default(),
                    None => return Err(err!("ad.groupNotFound", g)),
                }
            };
            let dn = filter::escape(&dn, false);
            parts.push(if o.recursive { format!("(memberOf:{}:={dn})", filter::RULE_CHAIN) } else { format!("(memberOf={dn})") });
        }
        let post = o.locked_out.is_some() || o.password_expired.is_some();
        let (ask, order) = self.ask(&o.properties);
        let q = Query { base: self.base(&o.base), scope: o.scope, filter: filter::and(&parts), attrs: ask, limit: if post { 0 } else { o.limit } };
        let mut out = Vec::new();
        self.query(&q, &mut |e| {
            let obj = Self::object(e, &order);
            let flag = |k: &str| obj.iter().find(|(n, _)| n == k).map(|(_, v)| *v == Val::Bool(true)).unwrap_or(false);
            if o.locked_out.is_some_and(|x| x != flag("lockedOut")) || o.password_expired.is_some_and(|x| x != flag("passwordExpired")) {
                return true;
            }
            out.push(obj);
            o.limit == 0 || out.len() < o.limit
        })?;
        Ok(out)
    }

    /// One object by its distinguished name, sAMAccountName ("DOMAIN\name"
    /// too; computers with or without "$"), userPrincipalName or mail,
    /// SID, GUID or name.
    pub fn get(&self, identity: &str, types: &[String], props: &[String]) -> Result<Option<Obj>> {
        let id = identity.trim();
        if id.is_empty() {
            return Ok(None);
        }
        let mut parts: Vec<String> = types_filter(types)?.into_iter().collect();
        let (ask, order) = self.ask(props);
        let mut q = Query { base: self.root.default_nc.clone(), scope: Scope::Subtree, filter: String::new(), attrs: ask, limit: 0 };
        let mut rank: Vec<String> = Vec::new();
        if is_dn(id) {
            q.base = id.to_string();
            q.scope = Scope::Base;
        } else if let Some(b) = value::sid_bytes(id) {
            parts.push(format!("(objectSid={})", filter::escape_bytes(&b)));
        } else if let Some(b) = value::guid_bytes(id) {
            parts.push(format!("(objectGUID={})", filter::escape_bytes(&b)));
        } else if id.contains('@') {
            let v = filter::escape(id, false);
            parts.push(format!("(|(userPrincipalName={v})(mail={v}))"));
        } else {
            let sam = id.rsplit_once('\\').map(|(_, s)| s).unwrap_or(id);
            let v = filter::escape(sam, false);
            let mut alts = vec![format!("(sAMAccountName={v})")];
            rank.push(sam.to_lowercase());
            if !sam.ends_with('$') {
                alts.push(format!("(sAMAccountName={v}$)"));
                rank.push(format!("{}$", sam.to_lowercase()));
            }
            if !id.contains('\\') {
                alts.push(format!("(name={v})"));
            }
            parts.push(filter::or(&alts));
        }
        q.filter = filter::and(&parts);
        let mut found: Vec<Obj> = Vec::new();
        let r = self.query(&q, &mut |e| {
            found.push(Self::object(e, &order));
            found.len() < 20
        });
        match r {
            Err(e) if q.scope == Scope::Base && e.key() == Some("ad.noSuchObject") => return Ok(None),
            r => r?,
        }
        let score = |o: &Obj| {
            let sam = o.iter().find(|(k, _)| k == "sAMAccountName").and_then(|(_, v)| if let Val::Str(s) = v { Some(s.to_lowercase()) } else { None });
            sam.and_then(|s| rank.iter().position(|r| *r == s)).unwrap_or(rank.len())
        };
        found.sort_by_key(score);
        Ok(found.into_iter().next())
    }

    /// The signed-in user (or this computer).
    pub fn me(&self, computer: bool, props: &[String]) -> Result<Obj> {
        let dn = self_dn(computer)?;
        self.get(&dn, &[], props)?.ok_or_else(|| err!("ad.noSuchObject", dn))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(o: &Obj, k: &str) -> Val {
        o.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or(Val::Null)
    }

    fn names(v: &[Obj]) -> Vec<String> {
        v.iter().map(|o| if let Val::Str(n) = s(o, "sAMAccountName") { n } else { String::new() }).collect()
    }

    #[test]
    fn helpers() {
        assert_eq!(dns_name("DC=corp,DC=example,DC=com"), "corp.example.com");
        assert!(is_dn("CN=山田 太郎,OU=営業部,DC=corp,DC=example,DC=com"));
        assert!(!is_dn("yamada"));
        assert!(!is_dn("a=b"));
        assert!(!is_dn("CORP\\yamada"));
    }

    #[test]
    fn search_and_get() {
        let _g = crate::test_lang("ja");
        let ses = Session::open("").unwrap();
        let d = ses.domain().unwrap();
        assert_eq!(s(&d, "name"), Val::Str("corp.example.com".into()));
        assert_eq!(s(&d, "netbiosName"), Val::Str("CORP".into()));
        assert_eq!(s(&d, "server"), Val::Str("dc01.corp.example.com".into()));

        let all = ses.search(&SearchOptions::default()).unwrap();
        assert_eq!(names(&all), ["yamada", "suzuki", "tanaka", "sato"]);
        let disabled = ses.search(&SearchOptions { disabled: Some(true), ..Default::default() }).unwrap();
        assert_eq!(names(&disabled), ["tanaka"]);
        let locked = ses.search(&SearchOptions { locked_out: Some(true), ..Default::default() }).unwrap();
        assert_eq!(names(&locked), ["suzuki"]);
        let pc = ses.search(&SearchOptions { types: vec!["computer".into()], ..Default::default() }).unwrap();
        assert_eq!(names(&pc), ["PC001$", "PC002$"]);
        let by_name = ses.search(&SearchOptions { name: Some("山田*".into()), ..Default::default() }).unwrap();
        assert_eq!(names(&by_name), ["yamada"]);
        let conds = SearchOptions {
            conds: vec![("department".into(), Cond::Any(vec![Cond::Eq("営業部".into()), Cond::Eq("総務部".into())]))],
            ..Default::default()
        };
        assert_eq!(names(&ses.search(&conds).unwrap()), ["yamada", "suzuki", "sato"]);
        let members = SearchOptions { member_of: Some("営業部".into()), types: vec!["all".into()], ..Default::default() };
        assert_eq!(names(&ses.search(&members).unwrap()), ["yamada", "suzuki"]);
        let nested = SearchOptions { member_of: Some("全社員".into()), recursive: true, ..Default::default() };
        assert_eq!(names(&ses.search(&nested).unwrap()), ["yamada", "suzuki", "sato"]);
        let limited = ses.search(&SearchOptions { limit: 2, ..Default::default() }).unwrap();
        assert_eq!(limited.len(), 2);
        let ou = SearchOptions { base: "OU=総務部,DC=corp,DC=example,DC=com".into(), ..Default::default() };
        assert_eq!(names(&ses.search(&ou).unwrap()), ["sato"]);
        let e = ses.search(&SearchOptions { filters: vec!["(cn=a".into()], ..Default::default() }).unwrap_err();
        assert!(e.to_string().contains("検索条件"), "{e}");
        let e = ses.search(&SearchOptions { types: vec!["printer".into()], ..Default::default() }).unwrap_err();
        assert!(e.to_string().contains("printer"), "{e}");

        let y = ses.get("yamada", &[], &[]).unwrap().unwrap();
        assert_eq!(s(&y, "type"), Val::Str("user".into()));
        assert_eq!(s(&y, "enabled"), Val::Bool(true));
        assert_eq!(s(&y, "objectSid"), Val::Str("S-1-5-21-1-2-3-1105".into()));
        assert!(matches!(s(&y, "whenCreated"), Val::Date(_)));
        assert_eq!(ses.get("CORP\\yamada", &[], &[]).unwrap().unwrap()[1], y[1]);
        assert_eq!(ses.get("yamada@corp.example.com", &[], &[]).unwrap().unwrap()[1], y[1]);
        assert_eq!(ses.get("S-1-5-21-1-2-3-1105", &[], &[]).unwrap().unwrap()[1], y[1]);
        assert_eq!(ses.get("CN=山田 太郎,OU=営業部,DC=corp,DC=example,DC=com", &[], &[]).unwrap().unwrap()[1], y[1]);
        assert!(ses.get("CN=いない,DC=corp,DC=example,DC=com", &[], &[]).unwrap().is_none());
        assert!(ses.get("nobody", &[], &[]).unwrap().is_none());
        let pc = ses.get("PC001", &["computer".into()], &[]).unwrap().unwrap();
        assert_eq!(s(&pc, "dNSHostName"), Val::Str("pc001.corp.example.com".into()));
        let g = ses.get("営業部", &["group".into()], &["member".into()]).unwrap().unwrap();
        assert_eq!(s(&g, "groupScope"), Val::Str("global".into()));
        assert_eq!(s(&g, "groupCategory"), Val::Str("security".into()));
        assert!(matches!(s(&g, "member"), Val::List(v) if v.len() == 2));

        let me = ses.me(false, &["tokenGroups".into()]).unwrap();
        assert_eq!(s(&me, "sAMAccountName"), Val::Str("yamada".into()));
        assert!(matches!(s(&me, "tokenGroups"), Val::List(v) if v.len() == 2));
        assert_eq!(s(&ses.me(true, &[]).unwrap(), "name"), Val::Str("PC001".into()));
    }
}
