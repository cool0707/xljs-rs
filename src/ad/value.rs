//! Attribute values of Active Directory as script values: dates from the
//! FILETIME numbers and generalized times, SIDs and GUIDs as strings,
//! numbers, and arrays for the attributes that hold several values.

/// A value handed to the script.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    /// A Date (milliseconds since 1970, UTC).
    Date(f64),
    Bytes(Vec<u8>),
    List(Vec<Val>),
}

/// Attributes that hold a FILETIME (100 ns since 1601) as a number. 0 and
/// the largest value mean "never" (null).
const FILETIME: &[&str] = &[
    "accountExpires", "badPasswordTime", "lastLogoff", "lastLogon", "lastLogonTimestamp", "lockoutTime",
    "msDS-UserPasswordExpiryTimeComputed", "pwdLastSet",
];

const INTEGER: &[&str] = &[
    "adminCount", "badPwdCount", "codePage", "countryCode", "groupType", "instanceType", "logonCount",
    "msDS-SupportedEncryptionTypes", "msDS-User-Account-Control-Computed", "primaryGroupID", "sAMAccountType",
    "systemFlags", "userAccountControl",
];

const BOOLEAN: &[&str] = &["isCriticalSystemObject", "isDeleted", "showInAdvancedViewOnly"];

const GUID: &[&str] = &["msExchMailboxGuid", "mS-DS-ConsistencyGuid", "objectGUID"];

const SID: &[&str] = &["objectSid", "securityIdentifier", "sIDHistory", "tokenGroups"];

/// Binary attributes kept as bytes even when they happen to be valid UTF-8.
const BINARY: &[&str] = &["jpegPhoto", "logonHours", "nTSecurityDescriptor", "thumbnailPhoto", "userCertificate"];

/// Attributes that hold several values: always arrays (empty when missing
/// and asked for).
pub const MULTI: &[&str] = &[
    "directReports", "dSCorePropagationData", "managedObjects", "member", "memberOf", "objectClass", "otherHomePhone",
    "otherMailbox", "otherMobile", "otherTelephone", "proxyAddresses", "servicePrincipalName", "sIDHistory", "tokenGroups",
    "url", "userCertificate",
];

pub fn is_in(list: &[&str], name: &str) -> bool {
    list.iter().any(|a| a.eq_ignore_ascii_case(name))
}

/// The values of an attribute as one script value.
pub fn convert(name: &str, raw: &[Vec<u8>]) -> Val {
    let vals: Vec<Val> = raw.iter().map(|b| one(name, b)).collect();
    if is_in(MULTI, name) || vals.len() > 1 {
        return Val::List(vals);
    }
    vals.into_iter().next().unwrap_or(Val::Null)
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn one(name: &str, b: &[u8]) -> Val {
    if is_in(GUID, name) && b.len() == 16 {
        return Val::Str(guid(b));
    }
    if is_in(SID, name) {
        if let Some(s) = sid(b) {
            return Val::Str(s);
        }
    }
    if is_in(BINARY, name) {
        return Val::Bytes(b.to_vec());
    }
    let Ok(s) = std::str::from_utf8(b) else { return Val::Bytes(b.to_vec()) };
    if is_in(FILETIME, name) {
        return match s.trim().parse::<i64>() {
            Ok(ft) => filetime(ft).map(Val::Date).unwrap_or(Val::Null),
            Err(_) => Val::Str(s.into()),
        };
    }
    if is_in(INTEGER, name) {
        if let Ok(n) = s.trim().parse::<i64>() {
            return Val::Num(n as f64);
        }
    }
    if is_in(BOOLEAN, name) {
        match s {
            "TRUE" => return Val::Bool(true),
            "FALSE" => return Val::Bool(false),
            _ => {}
        }
    }
    if let Some(ms) = generalized_time(s) {
        return Val::Date(ms);
    }
    Val::Str(text(b))
}

/// Milliseconds between 1601-01-01 and 1970-01-01.
const EPOCH_DIFF_MS: i64 = 11_644_473_600_000;

/// A FILETIME as milliseconds since 1970 (None for 0 and "never").
pub fn filetime(ft: i64) -> Option<f64> {
    if ft <= 0 || ft == i64::MAX {
        return None;
    }
    Some((ft / 10_000 - EPOCH_DIFF_MS) as f64)
}

/// Milliseconds since 1970 as a FILETIME.
pub fn to_filetime(ms: i64) -> i64 {
    (ms + EPOCH_DIFF_MS) * 10_000
}

/// "20240401093000.0Z" as milliseconds since 1970.
pub fn generalized_time(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    if b.len() < 15 || !b[..14].iter().all(u8::is_ascii_digit) || !s.ends_with('Z') {
        return None;
    }
    let frac = &s[14..s.len() - 1];
    if !(frac.is_empty() || (frac.starts_with('.') && frac[1..].bytes().all(|c| c.is_ascii_digit()))) {
        return None;
    }
    let dt = chrono::NaiveDateTime::parse_from_str(&s[..14], "%Y%m%d%H%M%S").ok()?;
    let ms = frac.get(1..).filter(|f| !f.is_empty()).map(|f| format!("{f:0<3}")[..3].parse::<i64>().unwrap_or(0)).unwrap_or(0);
    Some((dt.and_utc().timestamp_millis() + ms) as f64)
}

/// A GUID in its usual text form (the first three groups are little-endian).
pub fn guid(b: &[u8]) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{}",
        u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        u16::from_le_bytes([b[4], b[5]]),
        u16::from_le_bytes([b[6], b[7]]),
        b[8],
        b[9],
        b[10..16].iter().map(|x| format!("{x:02x}")).collect::<String>()
    )
}

/// The text of a GUID as its 16 bytes.
pub fn guid_bytes(s: &str) -> Option<Vec<u8>> {
    let h: String = s.trim_matches(|c| c == '{' || c == '}').split('-').collect();
    let parts: Vec<&str> = s.trim_matches(|c| c == '{' || c == '}').split('-').collect();
    if h.len() != 32 || parts.iter().map(|p| p.len()).collect::<Vec<_>>() != [8, 4, 4, 4, 12] {
        return None;
    }
    let mut b: Vec<u8> = (0..16).map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16)).collect::<Result<_, _>>().ok()?;
    b[0..4].reverse();
    b[4..6].reverse();
    b[6..8].reverse();
    Some(b)
}

/// A binary SID as "S-1-5-21-...".
pub fn sid(b: &[u8]) -> Option<String> {
    if b.len() < 8 || b.len() != 8 + 4 * b[1] as usize {
        return None;
    }
    let auth = b[2..8].iter().fold(0u64, |a, &x| a << 8 | x as u64);
    let mut s = format!("S-{}-{auth}", b[0]);
    for c in b[8..].chunks(4) {
        s.push_str(&format!("-{}", u32::from_le_bytes([c[0], c[1], c[2], c[3]])));
    }
    Some(s)
}

/// "S-1-5-21-..." as a binary SID.
pub fn sid_bytes(s: &str) -> Option<Vec<u8>> {
    let mut it = s.strip_prefix("S-").or_else(|| s.strip_prefix("s-"))?.split('-');
    let rev: u8 = it.next()?.parse().ok()?;
    let auth: u64 = it.next()?.parse().ok()?;
    let subs: Vec<u32> = it.map(|x| x.parse()).collect::<Result<_, _>>().ok()?;
    if subs.len() > 15 || auth >= 1 << 48 {
        return None;
    }
    let mut b = vec![rev, subs.len() as u8];
    b.extend_from_slice(&auth.to_be_bytes()[2..]);
    for x in subs {
        b.extend_from_slice(&x.to_le_bytes());
    }
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts() {
        let sid_b = sid_bytes("S-1-5-21-1004336348-1177238915-682003330-512").unwrap();
        assert_eq!(sid(&sid_b).unwrap(), "S-1-5-21-1004336348-1177238915-682003330-512");
        assert_eq!(convert("objectSid", &[sid_b]), Val::Str("S-1-5-21-1004336348-1177238915-682003330-512".into()));
        let g = guid_bytes("{3f2504e0-4f89-11d3-9a0c-0305e82c3301}").unwrap();
        assert_eq!(g[0..4], [0xe0, 0x04, 0x25, 0x3f]);
        assert_eq!(convert("objectGUID", &[g]), Val::Str("3f2504e0-4f89-11d3-9a0c-0305e82c3301".into()));
        assert_eq!(guid_bytes("3f2504e0-4f89-11d3-9a0c"), None);
        // 2024-04-01T00:00:00Z
        let ms = 1_711_929_600_000i64;
        assert_eq!(convert("pwdLastSet", &[to_filetime(ms).to_string().into_bytes()]), Val::Date(ms as f64));
        assert_eq!(convert("accountExpires", &[b"9223372036854775807".to_vec()]), Val::Null);
        assert_eq!(convert("lockoutTime", &[b"0".to_vec()]), Val::Null);
        assert_eq!(convert("whenCreated", &[b"20240401000000.0Z".to_vec()]), Val::Date(ms as f64));
        assert_eq!(generalized_time("20240401000000.5Z"), Some(ms as f64 + 500.0));
        assert_eq!(generalized_time("2024040100000Z"), None);
        assert_eq!(convert("userAccountControl", &[b"514".to_vec()]), Val::Num(514.0));
        assert_eq!(convert("employeeID", &[b"00123".to_vec()]), Val::Str("00123".into()));
        assert_eq!(convert("memberOf", &[b"CN=a".to_vec()]), Val::List(vec![Val::Str("CN=a".into())]));
        assert_eq!(convert("memberof", &[]), Val::List(vec![]));
        assert_eq!(convert("description", &[b"a".to_vec(), b"b".to_vec()]), Val::List(vec![Val::Str("a".into()), Val::Str("b".into())]));
        assert_eq!(convert("thumbnailPhoto", &[b"abc".to_vec()]), Val::Bytes(b"abc".to_vec()));
        assert_eq!(convert("x", &[vec![0xff, 0xfe]]), Val::Bytes(vec![0xff, 0xfe]));
        assert_eq!(convert("isCriticalSystemObject", &[b"TRUE".to_vec()]), Val::Bool(true));
    }
}
