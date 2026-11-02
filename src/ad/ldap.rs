//! The directory of the domain through wldap32: a connection to a domain
//! controller (the computer's domain unless a server is named), bound with
//! the credentials of the signed-in user (Kerberos/NTLM), and paged
//! searches.

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows::Win32::Networking::Ldap::*;
use windows::core::{HSTRING, PCWSTR, PWSTR};

use super::{Directory, Entry, Query, Scope};
use crate::{Error, Result, err};

const LDAP_AUTH_NEGOTIATE: u32 = 0x0486;
const LDAP_OPT_ON: *const c_void = 1 as *const c_void;
const LDAP_OPT_OFF: *const c_void = null();
const PAGE_SIZE: u32 = 500;
/// Seconds to wait for the domain controller to answer.
const CONNECT_TIMEOUT: i32 = 15;
const SEARCH_TIMEOUT: i32 = 120;

pub struct Ldap {
    ld: *mut LDAP,
    server: String,
}

fn message(code: u32) -> String {
    let p = unsafe { ldap_err2stringW(code) };
    if p.is_null() { String::new() } else { unsafe { p.to_string().unwrap_or_default() } }
}

/// The error for an LDAP result code; what names the object.
fn ldap_error(code: u32, server: &str, what: &str) -> Error {
    let c = LDAP_RETCODE(code as i32);
    if c == LDAP_SERVER_DOWN || c == LDAP_CONNECT_ERROR || c == LDAP_UNAVAILABLE {
        super::server_down(server)
    } else if c == LDAP_NO_SUCH_OBJECT || c == LDAP_INVALID_DN_SYNTAX {
        err!("ad.noSuchObject", what)
    } else if c == LDAP_FILTER_ERROR {
        err!("ad.badFilter", what)
    } else if c == LDAP_TIMEOUT || c == LDAP_TIMELIMIT_EXCEEDED {
        err!("ad.timeout")
    } else if c == LDAP_INVALID_CREDENTIALS || c == LDAP_STRONG_AUTH_REQUIRED || c == LDAP_INSUFFICIENT_RIGHTS {
        err!("ad.denied", message(code))
    } else {
        err!("ad.ldapError", code, message(code))
    }
}

fn last_error(server: &str, what: &str) -> Error {
    ldap_error(unsafe { LdapGetLastError() }, server, what)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

impl Ldap {
    /// Connects to server ("" for a domain controller of the computer's
    /// domain) and binds as the signed-in user.
    pub fn connect(server: &str) -> Result<Ldap> {
        let host = HSTRING::from(server);
        let ld = unsafe { ldap_initW(if server.is_empty() { PCWSTR::null() } else { PCWSTR(host.as_ptr()) }, LDAP_PORT) };
        if ld.is_null() {
            return Err(last_error(server, server));
        }
        let l = Ldap { ld, server: server.to_string() };
        unsafe {
            let version: u32 = LDAP_VERSION3;
            ldap_set_optionW(ld, LDAP_OPT_PROTOCOL_VERSION as i32, &version as *const u32 as *const c_void);
            // Referrals to other domains would be followed with long waits.
            ldap_set_optionW(ld, LDAP_OPT_REFERRALS as i32, LDAP_OPT_OFF);
            // Domain controllers that require LDAP signing reject unsigned binds.
            ldap_set_optionW(ld, LDAP_OPT_SIGN as i32, LDAP_OPT_ON);
            let limit: u32 = SEARCH_TIMEOUT as u32;
            ldap_set_optionW(ld, LDAP_OPT_TIMELIMIT as i32, &limit as *const u32 as *const c_void);
            let mut tv = LDAP_TIMEVAL { tv_sec: CONNECT_TIMEOUT, tv_usec: 0 };
            let rc = ldap_connect(ld, &mut tv);
            if rc != 0 {
                return Err(ldap_error(rc, server, server));
            }
            let rc = ldap_bind_sW(ld, PCWSTR::null(), PCWSTR::null(), LDAP_AUTH_NEGOTIATE);
            if rc != 0 {
                return Err(ldap_error(rc, server, server));
            }
        }
        Ok(l)
    }

    /// The entries of a result message; false from each stops.
    unsafe fn entries(&self, res: *mut LDAPMessage, each: &mut dyn FnMut(Entry) -> bool) -> bool {
        unsafe {
            let mut e = ldap_first_entry(self.ld, res);
            while !e.is_null() {
                if !each(self.entry(e)) {
                    return false;
                }
                e = ldap_next_entry(self.ld, e);
            }
        }
        true
    }

    unsafe fn entry(&self, e: *mut LDAPMessage) -> Entry {
        unsafe {
            let p = ldap_get_dnW(self.ld, e);
            let dn = if p.is_null() { String::new() } else { p.to_string().unwrap_or_default() };
            if !p.is_null() {
                ldap_memfreeW(PCWSTR(p.0));
            }
            let mut attrs = Vec::new();
            let mut ber: *mut BerElement = null_mut();
            let mut a: PWSTR = ldap_first_attributeW(self.ld, e, &mut ber);
            while !a.is_null() {
                let name = a.to_string().unwrap_or_default();
                let mut vals = Vec::new();
                let vp = ldap_get_values_lenW(self.ld, e, PCWSTR(a.0));
                if !vp.is_null() {
                    let mut i = 0;
                    loop {
                        let bv = *vp.add(i);
                        if bv.is_null() {
                            break;
                        }
                        let bv = &*bv;
                        vals.push(if bv.bv_len == 0 || bv.bv_val.is_null() {
                            Vec::new()
                        } else {
                            std::slice::from_raw_parts(bv.bv_val.0, bv.bv_len as usize).to_vec()
                        });
                        i += 1;
                    }
                    ldap_value_free_len(vp);
                }
                ldap_memfreeW(PCWSTR(a.0));
                attrs.push((name, vals));
                a = ldap_next_attributeW(self.ld, e, ber);
            }
            if !ber.is_null() {
                ber_free(ber, 0);
            }
            Entry { dn, attrs }
        }
    }
}

impl Drop for Ldap {
    fn drop(&mut self) {
        unsafe { ldap_unbind(self.ld) };
    }
}

impl Directory for Ldap {
    fn search(&self, q: &Query, each: &mut dyn FnMut(Entry) -> bool) -> Result<()> {
        let base = HSTRING::from(q.base.as_str());
        let filter = HSTRING::from(q.filter.as_str());
        let wides: Vec<Vec<u16>> = q.attrs.iter().map(|a| wide(a)).collect();
        let mut attrs: Vec<*const u16> = wides.iter().map(|w| w.as_ptr()).collect();
        attrs.push(null());
        let scope = match q.scope {
            Scope::Base => LDAP_SCOPE_BASE,
            Scope::OneLevel => LDAP_SCOPE_ONELEVEL,
            Scope::Subtree => LDAP_SCOPE_SUBTREE,
        };
        let (server, what) = (self.server.as_str(), q.base.as_str());
        let limit = q.limit.min(u32::MAX as usize) as u32;
        unsafe {
            if q.scope == Scope::Base {
                // The root DSE and single objects: no paging.
                let mut res: *mut LDAPMessage = null_mut();
                let mut tv = LDAP_TIMEVAL { tv_sec: SEARCH_TIMEOUT, tv_usec: 0 };
                let rc = ldap_search_ext_sW(self.ld, &base, scope, &filter, attrs.as_ptr(), 0, None, None, &mut tv, limit, &mut res);
                if rc != 0 && rc != LDAP_SIZELIMIT_EXCEEDED.0 as u32 {
                    if !res.is_null() {
                        ldap_msgfree(res);
                    }
                    return Err(ldap_error(rc, server, what));
                }
                if !res.is_null() {
                    self.entries(res, each);
                    ldap_msgfree(res);
                }
                return Ok(());
            }
            let s = ldap_search_init_pageW(self.ld, &base, scope, &filter, attrs.as_ptr(), 0, null_mut(), null_mut(), SEARCH_TIMEOUT as u32, limit, null_mut());
            if s.0 == 0 {
                return Err(last_error(server, what));
            }
            let mut result = Ok(());
            loop {
                let mut res: *mut LDAPMessage = null_mut();
                let mut total = 0u32;
                let mut tv = LDAP_TIMEVAL { tv_sec: SEARCH_TIMEOUT, tv_usec: 0 };
                let rc = ldap_get_next_page_s(self.ld, s, &mut tv, PAGE_SIZE, &mut total, &mut res);
                if rc == LDAP_NO_RESULTS_RETURNED.0 as u32 {
                    if !res.is_null() {
                        ldap_msgfree(res);
                    }
                    break;
                }
                if rc != 0 && rc != LDAP_SIZELIMIT_EXCEEDED.0 as u32 {
                    if !res.is_null() {
                        ldap_msgfree(res);
                    }
                    result = Err(ldap_error(rc, server, what));
                    break;
                }
                let go_on = res.is_null() || self.entries(res, each);
                if !res.is_null() {
                    ldap_msgfree(res);
                }
                if !go_on || rc != 0 {
                    break;
                }
            }
            ldap_search_abandon_page(self.ld, s);
            result
        }
    }
}

/// Whether the computer is joined to a domain.
pub fn joined() -> bool {
    use windows::Win32::NetworkManagement::NetManagement::*;
    unsafe {
        let mut name = PWSTR::null();
        let mut status = NETSETUP_JOIN_STATUS(0);
        let rc = NetGetJoinInformation(PCWSTR::null(), &mut name, &mut status);
        if !name.is_null() {
            NetApiBufferFree(Some(name.0 as *const c_void));
        }
        rc == 0 && status == NetSetupDomainName
    }
}

/// The distinguished name of the signed-in user (or of this computer).
pub fn self_dn(computer: bool) -> Result<String> {
    use windows::Win32::Security::Authentication::Identity::*;
    let call = |buf: Option<PWSTR>, n: &mut u32| unsafe {
        if computer { GetComputerObjectNameW(NameFullyQualifiedDN, buf, n) } else { GetUserNameExW(NameFullyQualifiedDN, buf, n) }
    };
    let fail = || if computer || !joined() { err!("ad.notJoined") } else { err!("ad.notDomainUser") };
    let mut n = 0u32;
    call(None, &mut n);
    if n == 0 {
        return Err(fail());
    }
    let mut buf = vec![0u16; n as usize + 1];
    let mut n = buf.len() as u32;
    if !call(Some(PWSTR(buf.as_mut_ptr())), &mut n) {
        return Err(fail());
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(String::from_utf16_lossy(&buf[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The calls work on any Windows: a computer outside a domain (the CI
    /// runners) has no computer object.
    #[test]
    fn join_state() {
        let _g = crate::test_lang("ja");
        match self_dn(true) {
            Ok(dn) => assert!(joined() && dn.contains("DC="), "{dn}"),
            Err(e) => assert!(!joined() || e.key() == Some("ad.notJoined"), "{e}"),
        }
    }
}
