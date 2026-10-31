//! Number formats: the text a cell shows (Excel's formatting of numbers and
//! dates with a format code), for the common codes.

fn sections(code: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut in_quote = false;
    let mut esc = false;
    for c in code.chars() {
        if esc {
            esc = false;
            out.last_mut().unwrap().push(c);
            continue;
        }
        match c {
            '\\' => {
                esc = true;
                out.last_mut().unwrap().push(c);
            }
            '"' => {
                in_quote = !in_quote;
                out.last_mut().unwrap().push(c);
            }
            ';' if !in_quote => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out
}

/// Removes [Red], [$-411] and similar bracketed parts (not [h] etc.).
fn strip_brackets(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    let mut in_quote = false;
    while let Some(c) = chars.next() {
        if c == '"' {
            in_quote = !in_quote;
        }
        if c == '[' && !in_quote {
            let mut inner = String::new();
            for d in chars.by_ref() {
                if d == ']' {
                    break;
                }
                inner.push(d);
            }
            let l = inner.to_lowercase();
            if l.chars().all(|c| matches!(c, 'h' | 'm' | 's')) && !l.is_empty() {
                out.push('[');
                out.push_str(&inner);
                out.push(']');
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// General: up to 11 significant digits, like Excel.
pub fn general(f: f64) -> String {
    if f == f.trunc() && f.abs() < 1e15 {
        return format!("{}", f as i64);
    }
    let a = f.abs();
    if (1e-9..1e15).contains(&a) {
        let digits = 10 - a.log10().floor() as i32;
        let s = format!("{:.*}", digits.clamp(0, 15) as usize, f);
        let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
        return s;
    }
    format!("{f:E}")
}

/// Formats a number with a (non-date) format code.
pub fn format_number(f: f64, code: &str) -> String {
    let secs = sections(code);
    let (sec, val, neg_sign) = if secs.len() >= 2 && f < 0.0 {
        (secs[1].clone(), -f, false)
    } else if secs.len() >= 3 && f == 0.0 {
        (secs[2].clone(), f, false)
    } else {
        (secs[0].clone(), f, f < 0.0)
    };
    let sec = strip_brackets(&sec);
    if sec.eq_ignore_ascii_case("general") || sec.is_empty() {
        return general(f);
    }
    if sec == "@" {
        return general(f);
    }
    // Split into literal parts and the number part.
    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut num = String::new();
    let mut seen_digit = false;
    let mut chars = sec.chars().peekable();
    let mut percent = 0;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut lit = String::new();
                for d in chars.by_ref() {
                    if d == '"' {
                        break;
                    }
                    lit.push(d);
                }
                if seen_digit { suffix.push_str(&lit) } else { prefix.push_str(&lit) }
            }
            '\\' => {
                if let Some(d) = chars.next() {
                    if seen_digit { suffix.push(d) } else { prefix.push(d) }
                }
            }
            '_' => {
                chars.next();
                if seen_digit { suffix.push(' ') } else { prefix.push(' ') }
            }
            '*' => {
                chars.next();
            }
            '0' | '#' | '?' | ',' | '.' if !(seen_digit && !suffix.is_empty()) => {
                if matches!(c, '0' | '#' | '?') {
                    seen_digit = true;
                }
                if c == ',' && !seen_digit {
                    prefix.push(c);
                } else {
                    num.push(c);
                }
            }
            'E' | 'e' if seen_digit => {
                // Scientific: fall back to Rust's exponent format.
                let dec = num.split('.').nth(1).map(|d| d.len()).unwrap_or(0);
                let s = format!("{:.*E}", dec, val);
                let (m, e) = s.split_once('E').unwrap();
                let e: i32 = e.parse().unwrap_or(0);
                let sign = if e < 0 { '-' } else { '+' };
                return format!("{}{prefix}{m}E{sign}{:02}{suffix}", if neg_sign { "-" } else { "" }, e.abs());
            }
            '%' => {
                percent += 1;
                if seen_digit { suffix.push('%') } else { prefix.push('%') }
            }
            _ => {
                if seen_digit { suffix.push(c) } else { prefix.push(c) }
            }
        }
    }
    let mut v = val.abs() * 100f64.powi(percent);
    // Trailing commas scale by 1000.
    while num.ends_with(',') {
        num.pop();
        v /= 1000.0;
    }
    let (int_pat, dec_pat) = match num.split_once('.') {
        Some((i, d)) => (i.to_string(), d.to_string()),
        None => (num.clone(), String::new()),
    };
    let grouping = int_pat.contains(',');
    let int_pat: String = int_pat.chars().filter(|c| *c != ',').collect();
    let min_dec = dec_pat.chars().filter(|c| *c == '0').count();
    let max_dec = dec_pat.chars().filter(|c| matches!(c, '0' | '#' | '?')).count();
    let s = format!("{:.*}", max_dec, v);
    let (ip, mut dp) = match s.split_once('.') {
        Some((i, d)) => (i.to_string(), d.to_string()),
        None => (s.clone(), String::new()),
    };
    while dp.len() > min_dec && dp.ends_with('0') {
        dp.pop();
    }
    let min_int = int_pat.chars().filter(|c| *c == '0').count();
    let mut ip = if ip == "0" && min_int == 0 { String::new() } else { ip };
    while ip.len() < min_int {
        ip.insert(0, '0');
    }
    if grouping {
        let digits: Vec<char> = ip.chars().collect();
        let mut g = String::new();
        for (i, c) in digits.iter().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                g.push(',');
            }
            g.push(*c);
        }
        ip = g;
    }
    let mut out = String::new();
    if neg_sign && (v != 0.0 || !dp.is_empty()) {
        out.push('-');
    }
    out.push_str(&prefix);
    out.push_str(&ip);
    if !dp.is_empty() || dec_pat.contains('0') {
        out.push('.');
        out.push_str(&dp);
    } else if !dec_pat.is_empty() && !dp.is_empty() {
        out.push('.');
        out.push_str(&dp);
    }
    out.push_str(&suffix);
    out
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const DAYS_JA: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];

/// Formats a wall-clock time (y, month, d, h, mi, s, ms) with a date code.
pub fn format_date(t: (i32, u32, u32, u32, u32, u32, u32), code: &str) -> String {
    let (y, mo, d, h, mi, s, ms) = t;
    let sec = strip_brackets(sections(code).into_iter().next().unwrap_or_default().as_str());
    let weekday = chrono::NaiveDate::from_ymd_opt(y, mo, d).map(|dt| {
        use chrono::Datelike;
        dt.weekday().num_days_from_sunday() as usize
    });
    let ampm = sec.to_lowercase().contains("am/pm") || sec.to_lowercase().contains("a/p");
    let chars: Vec<char> = sec.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut last_was_hour = false;
    let run = |i: usize, c: char| chars[i..].iter().take_while(|x| x.eq_ignore_ascii_case(&c)).count();
    while i < chars.len() {
        let c = chars[i];
        let lc = c.to_ascii_lowercase();
        match lc {
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    out.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '\\' => {
                if i + 1 < chars.len() {
                    out.push(chars[i + 1]);
                }
                i += 2;
            }
            'y' => {
                let n = run(i, 'y');
                if n <= 2 { out.push_str(&format!("{:02}", y % 100)) } else { out.push_str(&format!("{y:04}")) }
                i += n;
            }
            'm' => {
                let n = run(i, 'm');
                // Minutes after hours or before seconds.
                let rest: String = chars[i + n..].iter().collect::<String>().to_lowercase();
                let next_is_sec = rest.trim_start_matches(|c: char| !c.is_ascii_alphabetic()).starts_with('s');
                if (last_was_hour || next_is_sec) && n <= 2 {
                    out.push_str(&if n == 2 { format!("{mi:02}") } else { mi.to_string() });
                } else {
                    match n {
                        1 => out.push_str(&mo.to_string()),
                        2 => out.push_str(&format!("{mo:02}")),
                        3 => out.push_str(&MONTHS[(mo as usize + 11) % 12][..3]),
                        5 => out.push_str(&MONTHS[(mo as usize + 11) % 12][..1]),
                        _ => out.push_str(MONTHS[(mo as usize + 11) % 12]),
                    }
                }
                last_was_hour = false;
                i += n;
            }
            'd' => {
                let n = run(i, 'd');
                match n {
                    1 => out.push_str(&d.to_string()),
                    2 => out.push_str(&format!("{d:02}")),
                    3 => out.push_str(weekday.map(|w| &DAYS[w][..3]).unwrap_or("")),
                    _ => out.push_str(weekday.map(|w| DAYS[w]).unwrap_or("")),
                }
                i += n;
            }
            'a' if chars[i..].iter().collect::<String>().to_lowercase().starts_with("aaa") => {
                let n = run(i, 'a');
                if let Some(w) = weekday {
                    out.push_str(if n >= 4 { &["日曜日", "月曜日", "火曜日", "水曜日", "木曜日", "金曜日", "土曜日"][w] } else { DAYS_JA[w] });
                }
                i += n;
            }
            'a' if chars[i..].iter().collect::<String>().to_lowercase().starts_with("am/pm") => {
                out.push_str(if h < 12 { "AM" } else { "PM" });
                i += 5;
            }
            'a' if chars[i..].iter().collect::<String>().to_lowercase().starts_with("a/p") => {
                out.push(if h < 12 { 'A' } else { 'P' });
                i += 3;
            }
            'h' => {
                let n = run(i, 'h');
                let hh = if ampm { if h % 12 == 0 { 12 } else { h % 12 } } else { h };
                out.push_str(&if n >= 2 { format!("{hh:02}") } else { hh.to_string() });
                last_was_hour = true;
                i += n;
            }
            's' => {
                let n = run(i, 's');
                out.push_str(&if n >= 2 { format!("{s:02}") } else { s.to_string() });
                i += n;
                // Fractions of seconds: .0, .00, .000
                if i < chars.len() && chars[i] == '.' {
                    let z = chars[i + 1..].iter().take_while(|c| **c == '0').count();
                    if z > 0 {
                        let frac = format!("{:03}", ms);
                        out.push('.');
                        out.push_str(&frac[..z.min(3)]);
                        i += 1 + z;
                    }
                }
                last_was_hour = false;
            }
            'g' | 'e' if lc == 'e' && false => {}
            '[' => {
                // [h]:mm elapsed hours (not expected for dates); keep the text.
                i += 1;
            }
            ']' => i += 1,
            '_' => {
                out.push(' ');
                i += 2;
            }
            '*' => i += 2,
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(format_number(1200.0, "#,##0"), "1,200");
        assert_eq!(format_number(1234567.891, "#,##0.00"), "1,234,567.89");
        assert_eq!(format_number(0.256, "0.0%"), "25.6%");
        assert_eq!(format_number(3.0, "0.0"), "3.0");
        assert_eq!(format_number(-5.0, "#,##0;[Red]-#,##0"), "-5");
        assert_eq!(format_number(800.5, "General"), "800.5");
        assert_eq!(format_number(12.0, "0\"円\""), "12円");
        assert_eq!(general(0.1 + 0.2), "0.3");
    }

    #[test]
    fn dates() {
        let t = (2026, 4, 1, 13, 5, 9, 0);
        assert_eq!(format_date(t, "yyyy/m/d"), "2026/4/1");
        assert_eq!(format_date(t, "yyyy/m/d h:mm"), "2026/4/1 13:05");
        assert_eq!(format_date(t, "yyyy\"年\"m\"月\"d\"日\"(aaa)"), "2026年4月1日(水)");
        assert_eq!(format_date(t, "h:mm:ss AM/PM"), "1:05:09 PM");
        assert_eq!(format_date(t, "mm-dd-yy"), "04-01-26");
    }
}
