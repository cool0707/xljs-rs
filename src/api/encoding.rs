use encoding_rs::{Encoding, EncoderResult};

use super::N;
use crate::{Result, err};

const BOM_UTF8: &[u8] = b"\xEF\xBB\xBF";

/// The canonical name of an encoding ("auto", "utf8", "utf8bom", "cp932",
/// "eucjp", "iso2022jp", "utf16le", "utf16be"; others as given).
pub fn norm_encoding(name: &str) -> String {
    let s = name.trim().to_lowercase().replace('_', "-");
    match s.as_str() {
        "" | "auto" => "auto",
        "utf8" | "utf-8" => "utf8",
        "utf8bom" | "utf-8bom" | "utf8-bom" | "utf-8-bom" | "utf-8-sig" | "utf8-sig" => "utf8bom",
        "cp932" | "shift-jis" | "shiftjis" | "sjis" | "windows-31j" | "ms932" | "x-sjis" | "ms-kanji" | "csshiftjis" => "cp932",
        "euc-jp" | "eucjp" => "eucjp",
        "iso-2022-jp" | "jis" => "iso2022jp",
        "utf16le" | "utf-16le" | "utf-16" => "utf16le",
        "utf16be" | "utf-16be" => "utf16be",
        _ => return s,
    }
    .to_string()
}

pub(crate) fn is_utf8_bom(name: &str) -> bool {
    norm_encoding(name) == "utf8bom"
}

fn text_encoding(norm: &str) -> Result<&'static Encoding> {
    Ok(match norm {
        "cp932" => encoding_rs::SHIFT_JIS,
        "eucjp" => encoding_rs::EUC_JP,
        "iso2022jp" => encoding_rs::ISO_2022_JP,
        "utf16le" => encoding_rs::UTF_16LE,
        "utf16be" => encoding_rs::UTF_16BE,
        _ => return Err(err!("data.unsupportedEncoding", norm)),
    })
}

/// Guesses the encoding of b: a BOM wins, then valid UTF-8, otherwise CP932
/// (Shift_JIS).
pub fn detect_encoding(b: &[u8]) -> &'static str {
    if b.starts_with(BOM_UTF8) {
        "utf8bom"
    } else if b.starts_with(&[0xFF, 0xFE]) {
        "utf16le"
    } else if b.starts_with(&[0xFE, 0xFF]) {
        "utf16be"
    } else if std::str::from_utf8(b).is_ok() {
        "utf8"
    } else {
        "cp932"
    }
}

/// Decodes b from the named encoding ("auto" detects it).
pub fn decode(b: &[u8], name: &str) -> Result<String> {
    let mut norm = norm_encoding(name);
    if norm == "auto" {
        norm = detect_encoding(b).to_string();
    }
    let mut b = b;
    match norm.as_str() {
        "utf8" | "utf8bom" => {
            let b = b.strip_prefix(BOM_UTF8).unwrap_or(b);
            return Ok(String::from_utf8_lossy(b).into_owned());
        }
        "utf16le" => b = b.strip_prefix(&[0xFF, 0xFE]).unwrap_or(b),
        "utf16be" => b = b.strip_prefix(&[0xFE, 0xFF]).unwrap_or(b),
        _ => {}
    }
    let enc = text_encoding(&norm)?;
    let (s, _) = enc.decode_without_bom_handling(b);
    Ok(s.into_owned())
}

/// Converts s to the given encoding. Characters that cannot be represented
/// are replaced with "?".
pub fn encode(s: &str, name: &str) -> Result<Vec<u8>> {
    let norm = norm_encoding(name);
    match norm.as_str() {
        "auto" | "utf8" => return Ok(s.as_bytes().to_vec()),
        "utf8bom" => {
            let mut out = BOM_UTF8.to_vec();
            out.extend_from_slice(s.as_bytes());
            return Ok(out);
        }
        "utf16le" | "utf16be" => {
            let le = norm == "utf16le";
            let mut out = if le { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
            for u in s.encode_utf16() {
                out.extend_from_slice(&if le { u.to_le_bytes() } else { u.to_be_bytes() });
            }
            return Ok(out);
        }
        _ => {}
    }
    let enc = text_encoding(&norm)?;
    let mut e = enc.new_encoder();
    let mut out = Vec::with_capacity(s.len() * 2);
    let mut rest = s;
    loop {
        let need = e.max_buffer_length_from_utf8_without_replacement(rest.len()).unwrap_or(rest.len() * 4 + 16);
        out.reserve(need);
        let (res, read) = e.encode_from_utf8_to_vec_without_replacement(rest, &mut out, true);
        rest = &rest[read..];
        match res {
            EncoderResult::InputEmpty => break,
            EncoderResult::OutputFull => continue,
            EncoderResult::Unmappable(_) => out.push(b'?'),
        }
    }
    Ok(out)
}

pub(super) fn install(n: &N) {
    n.set("decode", |_, a| {
        let s = decode(&a.bytes(0)?, &a.opt_str(1, "utf8")).map_err(|e| a.throw(&e))?;
        a.ret(s)
    });
    n.set("encode", |_, a| {
        let b = encode(&a.str(0), &a.opt_str(1, "utf8")).map_err(|e| a.throw(&e))?;
        super::js::array_buffer(&a.ctx, b)
    });
    n.set("detectEncoding", |_, a| a.ret(detect_encoding(&a.bytes(0)?)));
    n.set("normEncoding", |_, a| a.ret(norm_encoding(&a.str(0))));
}
