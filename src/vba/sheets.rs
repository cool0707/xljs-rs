#![allow(deprecated)] // quick-xml unescape_value
use std::collections::HashMap;
use std::io::{Read, Seek};

use quick_xml::events::Event;

/// Maps the code names of the sheets of an Office Open XML workbook
/// (Sheet1, the names of their VBA modules) to the sheet names, read from
/// `<sheetPr codeName>` of each sheet. Missing parts give an empty map.
pub fn sheet_code_names<R: Read + Seek>(zr: &mut zip::ZipArchive<R>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let names: HashMap<String, String> = zr.file_names().map(|n| (n.to_lowercase(), n.to_string())).collect();
    let mut open = |name: &str| -> Option<Vec<u8>> {
        let real = names.get(&name.to_lowercase())?;
        let mut f = zr.by_name(real).ok()?;
        let mut b = Vec::new();
        f.read_to_end(&mut b).ok()?;
        Some(b)
    };
    const WB_PATH: &str = "xl/workbook.xml";
    let (Some(wb), Some(rels)) = (open(WB_PATH), open("xl/_rels/workbook.xml.rels")) else {
        return out;
    };
    let mut targets = HashMap::new();
    for (id, target) in elements(&rels, b"Relationship", &["Id", "Target"]).into_iter().map(|v| (v[0].clone(), v[1].clone())) {
        let t = match target.strip_prefix('/') {
            Some(t) => t.to_string(),
            None => crate::meta::clean_path(&format!("xl/{target}")),
        };
        targets.insert(id, t);
    }
    for v in elements(&wb, b"sheet", &["name", "r:id"]) {
        let Some(target) = targets.get(&v[1]) else { continue };
        if let Some(data) = open(target) {
            let code = sheet_code_name(&data);
            if !code.is_empty() {
                out.insert(code, v[0].clone());
            }
        }
    }
    out
}

/// The given attributes of every element with the local name, in order.
fn elements(xml: &[u8], local: &[u8], attrs: &[&str]) -> Vec<Vec<String>> {
    let mut r = quick_xml::Reader::from_reader(xml);
    let mut out = Vec::new();
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == local => {
                let mut vals = vec![String::new(); attrs.len()];
                for a in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
                    for (i, want) in attrs.iter().enumerate() {
                        // "r:id" matches any prefix bound to the relationships namespace.
                        let hit = if let Some(l) = want.strip_prefix("r:") {
                            key.ends_with(&format!(":{l}"))
                        } else {
                            key == *want
                        };
                        if hit {
                            vals[i] = a.unescape_value().map(|v| v.into_owned()).unwrap_or_default();
                        }
                    }
                }
                out.push(vals);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Reads `<sheetPr codeName>` at the top of a worksheet (or chartsheet)
/// part, without reading the cells.
fn sheet_code_name(xml: &[u8]) -> String {
    let mut r = quick_xml::Reader::from_reader(xml);
    let mut depth = 0;
    let mut buf = Vec::new();
    loop {
        let ev = r.read_event_into(&mut buf);
        match ev {
            Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                depth += 1;
                if depth == 2 {
                    if e.local_name().as_ref() != b"sheetPr" {
                        return String::new();
                    }
                    for a in e.attributes().flatten() {
                        if a.key.local_name().as_ref() == b"codeName" {
                            return a.unescape_value().map(|v| v.into_owned()).unwrap_or_default();
                        }
                    }
                    return String::new();
                }
                if matches!(ev, Ok(Event::Empty(_))) {
                    depth -= 1;
                }
            }
            Ok(Event::End(_)) => depth -= 1,
            Ok(Event::Eof) | Err(_) => return String::new(),
            _ => {}
        }
        buf.clear();
    }
}
