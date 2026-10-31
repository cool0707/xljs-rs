//! Position lookup in the bundle's inline source map.

use base64::Engine as _;

/// A source map decoded into per-line segments.
pub struct Mapper {
    sources: Vec<String>,
    /// For each generated line: (column, source, line, column), sorted.
    lines: Vec<Vec<(u32, u32, u32, u32)>>,
}

const MARKER: &str = "//# sourceMappingURL=data:application/json;base64,";

impl Mapper {
    /// The source map at the end of a bundle.
    pub fn from_bundle(code: &str) -> Option<Mapper> {
        let i = code.rfind(MARKER)?;
        let data = base64::engine::general_purpose::STANDARD.decode(code[i + MARKER.len()..].trim()).ok()?;
        let json = String::from_utf8(data).ok()?;
        let map = oxc_sourcemap::SourceMap::from_json_string(&json).ok()?;
        let sources = map.get_sources().map(str::to_string).collect();
        let mut lines: Vec<Vec<(u32, u32, u32, u32)>> = Vec::new();
        for t in map.get_tokens() {
            let Some(src) = t.get_source_id() else { continue };
            let l = t.get_dst_line() as usize;
            if lines.len() <= l {
                lines.resize_with(l + 1, Vec::new);
            }
            lines[l].push((t.get_dst_col(), src, t.get_src_line(), t.get_src_col()));
        }
        for l in &mut lines {
            l.sort();
        }
        Some(Mapper { sources, lines })
    }

    /// The original position of a 1-based line and column of the bundle:
    /// (source, line, column), 1-based.
    pub fn lookup(&self, line: u32, col: u32) -> Option<(String, u32, u32)> {
        let segs = self.lines.get(line.checked_sub(1)? as usize)?;
        let col0 = col.saturating_sub(1);
        let i = segs.partition_point(|s| s.0 <= col0);
        let s = if i == 0 { segs.first()? } else { &segs[i - 1] };
        Some((self.sources.get(s.1 as usize)?.clone(), s.2 + 1, s.3 + 1))
    }
}
