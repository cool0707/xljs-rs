//! The drawings of a sheet for the print layout (xlsx_print.rs): shapes,
//! text boxes, lines and connectors, groups and pictures as HTML/SVG, in
//! the order of the sheet's drawing, with the places of its charts.
//!
//! Only the common preset shapes are drawn as such (rectangles, rounded
//! rectangles, ellipses, triangles, diamonds, arrows, callouts, flowchart
//! symbols, stars, brackets ...); other presets fall back to their
//! rectangle. Freeforms (custGeom) are drawn from their paths.

use std::collections::HashMap;
use std::fmt::Write as _;

use base64::Engine as _;

use super::xlsx::Book;
use super::xlsx_chart::{Anchor, ChartParts, Node, anchor_of, color_of};
use super::xlsx_print::{esc, font_family, n};

const EMU_PT: f64 = 12700.0;

/// One object of a sheet's drawing.
pub enum Drawn {
    /// The index-th chart of sheet.charts.
    Chart(usize),
    /// The HTML of the object, filling a box of the anchor's size.
    Html(String),
}

pub struct DrawObj {
    pub anchor: Anchor,
    pub drawn: Drawn,
}

/// What drawing a shape needs.
struct Ctx<'a> {
    parts: &'a ChartParts,
    rels: HashMap<String, (String, String)>,
    theme: HashMap<String, String>,
    font: (String, f64),
    ids: &'a std::cell::Cell<usize>,
}

/// The objects of a sheet's drawing in drawing order (later ones on top),
/// or None when the workbook has no drawing parts (a new workbook).
///
/// size gives the size (pt) of an anchor's box on the sheet.
pub fn sheet_drawing(b: &Book, sheet: &str, ids: &std::cell::Cell<usize>, size: &dyn Fn(&Anchor) -> Option<(f64, f64)>) -> Option<Vec<DrawObj>> {
    let parts = b.charts.as_ref()?;
    if parts.files.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    let Some(sheet_part) = parts.sheet_part(sheet) else { return Some(out) };
    let mut drawings: Vec<String> = parts.rels(&sheet_part).into_values().filter(|(t, _)| t.ends_with("/drawing")).map(|(_, p)| p).collect();
    drawings.sort();
    let theme = parts.theme();
    let mut chart = 0;
    for d in drawings {
        let Some(drawing) = parts.xml(&d) else { continue };
        let ctx = Ctx { parts, rels: parts.rels(&d), theme: theme.clone(), font: b.default_font.clone(), ids };
        for a in drawing.children.iter().filter(|c| c.name.ends_with("Anchor")) {
            // Charts in the order of sheet_charts (xlsx_chart::each_chart).
            if let Some(frame) = a.find("graphicFrame")
                && let Some(data) = frame.find("graphicData")
                && let Some(id) = data.children.iter().find(|c| c.name == "chart").and_then(|c| c.attr("id"))
                && ctx.rels.contains_key(id)
            {
                out.push(DrawObj { anchor: anchor_of(a), drawn: Drawn::Chart(chart) });
                chart += 1;
                continue;
            }
            if a.child("clientData").and_then(|c| c.attr("fPrintsWithSheet")) == Some("0") {
                continue;
            }
            let Some(obj) = a.children.iter().find(|c| matches!(c.name.as_str(), "sp" | "cxnSp" | "grpSp" | "pic")).or_else(|| {
                // Shapes that newer Excel wraps in mc:AlternateContent.
                a.find("Fallback").and_then(|f| f.children.iter().find(|c| matches!(c.name.as_str(), "sp" | "cxnSp" | "grpSp" | "pic")))
            }) else {
                continue;
            };
            let anchor = anchor_of(a);
            // The box comes from the anchor (the shape's xfrm says the same in EMU).
            let ext = size(&anchor).or_else(|| xfrm(obj).map(|x| x.ext)).filter(|e| e.0 > 0.0 && e.1 > 0.0);
            let html = object_html(&ctx, obj, None, ext);
            if !html.is_empty() {
                out.push(DrawObj { anchor, drawn: Drawn::Html(html) });
            }
        }
    }
    Some(out)
}

/// The transform of a shape, picture or group (pt).
#[derive(Clone, Copy, Debug, Default)]
struct Xfrm {
    off: (f64, f64),
    ext: (f64, f64),
    ch_off: (f64, f64),
    ch_ext: (f64, f64),
    rot: f64,
    flip_h: bool,
    flip_v: bool,
}

fn xfrm(obj: &Node) -> Option<Xfrm> {
    let props = obj.children.iter().find(|c| matches!(c.name.as_str(), "spPr" | "grpSpPr"))?;
    let x = props.child("xfrm")?;
    let pair = |n: &str, a: &str, b: &str| -> (f64, f64) {
        x.child(n)
            .map(|e| {
                let g = |k: &str| e.attr(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / EMU_PT;
                (g(a), g(b))
            })
            .unwrap_or((0.0, 0.0))
    };
    let flag = |k: &str| matches!(x.attr(k), Some("1" | "true"));
    Some(Xfrm {
        off: pair("off", "x", "y"),
        ext: pair("ext", "cx", "cy"),
        ch_off: pair("chOff", "x", "y"),
        ch_ext: pair("chExt", "cx", "cy"),
        rot: x.attr("rot").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 60000.0,
        flip_h: flag("flipH"),
        flip_v: flag("flipV"),
    })
}

/// The HTML of a shape, connector, picture or group. Without place it
/// fills its container (a top-level object, sized by its anchor); in a
/// group, place is its box as fractions of the group's box. size is the
/// object's size in pt.
fn object_html(ctx: &Ctx, obj: &Node, place: Option<(f64, f64, f64, f64)>, size: Option<(f64, f64)>) -> String {
    let nv = obj.children.iter().find(|c| c.name.starts_with("nv")).and_then(|c| c.child("cNvPr"));
    if nv.and_then(|c| c.attr("hidden")).is_some_and(|h| h == "1" || h == "true") {
        return String::new();
    }
    let x = xfrm(obj).unwrap_or_default();
    let (w, h) = size.unwrap_or(x.ext);
    let pct = |v: f64| n(v * 100.0);
    let mut style = match place {
        Some((l, t, pw, ph)) => format!("left:{}%;top:{}%;width:{}%;height:{}%;", pct(l), pct(t), pct(pw), pct(ph)),
        None => "left:0;top:0;width:100%;height:100%;".to_string(),
    };
    if x.rot != 0.0 {
        let _ = write!(style, "transform:rotate({}deg);", n(x.rot));
    }
    let inner = match obj.name.as_str() {
        "grpSp" => group_html(ctx, obj, &x, w, h),
        "pic" => picture_html(ctx, obj, &x),
        _ => shape_html(ctx, obj, &x, w, h),
    };
    if inner.is_empty() {
        return String::new();
    }
    format!("<div class=\"xljs-shape\" style=\"{style}\">{inner}</div>")
}

fn group_html(ctx: &Ctx, g: &Node, x: &Xfrm, w: f64, h: f64) -> String {
    // Children are in the group's child coordinates (chOff, chExt).
    let (cw, ch) = (if x.ch_ext.0 > 0.0 { x.ch_ext.0 } else { x.ext.0.max(0.01) }, if x.ch_ext.1 > 0.0 { x.ch_ext.1 } else { x.ext.1.max(0.01) });
    let mut out = String::new();
    for c in g.children.iter().filter(|c| matches!(c.name.as_str(), "sp" | "cxnSp" | "grpSp" | "pic")) {
        let Some(cx) = xfrm(c) else { continue };
        let place = ((cx.off.0 - x.ch_off.0) / cw, (cx.off.1 - x.ch_off.1) / ch, cx.ext.0 / cw, cx.ext.1 / ch);
        out.push_str(&object_html(ctx, c, Some(place), Some((cx.ext.0 * w / cw, cx.ext.1 * h / ch))));
    }
    if x.flip_h || x.flip_v {
        return format!("<div style=\"position:absolute;inset:0;transform:scale({},{})\">{out}</div>", if x.flip_h { -1 } else { 1 }, if x.flip_v { -1 } else { 1 });
    }
    out
}

fn picture_html(ctx: &Ctx, pic: &Node, x: &Xfrm) -> String {
    let Some(id) = pic.find("blip").and_then(|b| b.attr("embed")) else { return String::new() };
    let Some((_, part)) = ctx.rels.get(id) else { return String::new() };
    let Some(data) = ctx.parts.files.get(part) else { return String::new() };
    let lower = part.to_lowercase();
    let mime = match lower.rsplit('.').next().unwrap_or("") {
        "png" => "image/png",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        _ => return String::new(), // EMF/WMF: browsers cannot show them
    };
    let flip = if x.flip_h || x.flip_v { format!("transform:scale({},{});", if x.flip_h { -1 } else { 1 }, if x.flip_v { -1 } else { 1 }) } else { String::new() };
    format!(
        "<img alt=\"\" src=\"data:{mime};base64,{}\" style=\"position:absolute;inset:0;width:100%;height:100%;{flip}\">",
        base64::engine::general_purpose::STANDARD.encode(data)
    )
}

/// A fill or line color: "none", a color, or None (not said).
fn paint(props: Option<&Node>, theme: &HashMap<String, String>) -> Option<(String, f64)> {
    let p = props?;
    if p.child("noFill").is_some() {
        return Some(("none".into(), 1.0));
    }
    let fill = p.child("solidFill").or_else(|| p.child("gradFill").and_then(|g| g.find("gs")))?;
    let color = color_of(fill, theme)?;
    let alpha = fill.children.first().and_then(|c| c.child("alpha")).and_then(|a| a.attr("val")).and_then(|v| v.parse::<f64>().ok()).map(|v| v / 100000.0).unwrap_or(1.0);
    Some((color, alpha))
}

/// The color of a style reference (lnRef, fillRef, fontRef) when its index is not 0.
fn style_ref(sp: &Node, name: &str, theme: &HashMap<String, String>) -> Option<(String, usize)> {
    let r = sp.child("style")?.child(name)?;
    let idx = r.attr("idx").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let color = color_of(r, theme)?;
    if name != "fontRef" && idx == 0 {
        return None;
    }
    Some((color, idx))
}

/// The adjust values of a preset (adj, adj1, adj2 ... in 1/100000).
fn adjusts(geom: &Node) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    if let Some(av) = geom.child("avLst") {
        for gd in av.all("gd") {
            if let (Some(name), Some(f)) = (gd.attr("name"), gd.attr("fmla"))
                && let Some(v) = f.strip_prefix("val ").and_then(|v| v.trim().parse::<f64>().ok())
            {
                out.insert(name.to_string(), v);
            }
        }
    }
    out
}

fn pts(p: &[(f64, f64)]) -> String {
    let mut s = String::new();
    for (i, (x, y)) in p.iter().enumerate() {
        let _ = write!(s, "{}{},{} ", if i == 0 { "M" } else { "L" }, n(*x), n(*y));
    }
    s.push('Z');
    s
}

/// The SVG path of a preset shape in a w × h box, and whether it is a line
/// (no fill). None for presets this does not know.
fn preset_path(prst: &str, w: f64, h: f64, adj: &HashMap<String, f64>) -> Option<(String, bool)> {
    let ss = w.min(h);
    let a = |k: &str, d: f64| adj.get(k).copied().unwrap_or(d) / 100000.0;
    let rect = pts(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]);
    let round_rect = |r: f64| {
        let r = r.min(w / 2.0).min(h / 2.0);
        format!(
            "M{r},0 L{x1},0 A{r},{r} 0 0 1 {w},{r} L{w},{y1} A{r},{r} 0 0 1 {x1},{h} L{r},{h} A{r},{r} 0 0 1 0,{y1} L0,{r} A{r},{r} 0 0 1 {r},0 Z",
            r = n(r),
            x1 = n(w - r),
            y1 = n(h - r),
            w = n(w),
            h = n(h)
        )
    };
    let ellipse = format!("M0,{cy} A{rx},{ry} 0 1 1 {w},{cy} A{rx},{ry} 0 1 1 0,{cy} Z", cy = n(h / 2.0), rx = n(w / 2.0), ry = n(h / 2.0), w = n(w));
    // Callouts: one outline with the tail in it, so that no line crosses
    // the tail's base. The tip is at adj1, adj2 from the center (of the size).
    let tip = (w / 2.0 + a("adj1", -20833.0) * w, h / 2.0 + a("adj2", 62500.0) * h);
    let box_callout = |r: f64| -> String {
        let r = r.min(w / 2.0).min(h / 2.0);
        let (dx, dy) = ((tip.0 - w / 2.0) / w, (tip.1 - h / 2.0) / h);
        // The edge the tail leaves from (none when the tip is inside).
        let edge = if dx.abs() <= 0.5 && dy.abs() <= 0.5 {
            ' '
        } else if dx.abs() > dy.abs() {
            if dx > 0.0 { 'r' } else { 'l' }
        } else if dy > 0.0 {
            'b'
        } else {
            't'
        };
        // The base of the tail on the edge: a quarter of it, on the tip's side.
        let span = |len: f64, toward_end: bool| if toward_end { (len * 7.0 / 12.0, len * 10.0 / 12.0) } else { (len * 2.0 / 12.0, len * 5.0 / 12.0) };
        let t = |e: char, p: f64, q: f64| -> String {
            if e != edge {
                return String::new();
            }
            let (a0, a1) = (p, q);
            let pt = |u: f64| match e {
                't' => (u, 0.0),
                'r' => (w, u),
                'b' => (u, h),
                _ => (0.0, u),
            };
            let (b1, b2) = (pt(a0), pt(a1));
            format!("L{},{} L{},{} L{},{} ", n(b1.0), n(b1.1), n(tip.0), n(tip.1), n(b2.0), n(b2.1))
        };
        let (tx0, tx1) = span(w, dx > 0.0);
        let (ty0, ty1) = span(h, dy > 0.0);
        let arc = |x: f64, y: f64| if r > 0.0 { format!("A{r},{r} 0 0 1 {},{} ", n(x), n(y), r = n(r)) } else { String::new() };
        format!(
            "M{},0 {}L{},0 {}{}L{},{} {}{}L{},{} {}{}L0,{} {}Z",
            n(r),
            t('t', tx0, tx1),
            n(w - r),
            arc(w, r),
            t('r', ty0, ty1),
            n(w),
            n(h - r),
            arc(w - r, h),
            t('b', tx1, tx0),
            n(r),
            n(h),
            arc(0.0, h - r),
            t('l', ty1, ty0),
            n(r),
            arc(r, 0.0),
        )
    };
    let ellipse_callout = || -> String {
        let (rx, ry) = (w / 2.0, h / 2.0);
        let (ux, uy) = ((tip.0 - rx) / rx.max(0.01), (tip.1 - ry) / ry.max(0.01));
        if ux * ux + uy * uy <= 1.0 {
            return ellipse.clone();
        }
        // The base: on the ellipse, either side of the direction of the tip.
        let th = uy.atan2(ux);
        let p = |a: f64| (rx + rx * a.cos(), ry + ry * a.sin());
        let (b1, b2) = (p(th + 0.3), p(th - 0.3));
        format!(
            "M{},{} A{},{} 0 1 1 {},{} L{},{} Z",
            n(b1.0),
            n(b1.1),
            n(rx),
            n(ry),
            n(b2.0),
            n(b2.1),
            n(tip.0),
            n(tip.1)
        )
    };
    let arrow = |dir: char| -> String {
        // Shaft thickness adj1 (of the cross size), head length adj2 (of the short side).
        let (len, cross) = if dir == 'r' || dir == 'l' { (w, h) } else { (h, w) };
        let th = cross * a("adj1", 50000.0);
        let head = (ss * a("adj2", 50000.0)).min(len);
        let (s0, s1) = ((cross - th) / 2.0, (cross + th) / 2.0);
        let p = [(0.0, s0), (len - head, s0), (len - head, 0.0), (len, cross / 2.0), (len - head, cross), (len - head, s1), (0.0, s1)];
        let map = |(u, v): (f64, f64)| match dir {
            'r' => (u, v),
            'l' => (w - u, v),
            'd' => (v, u),
            _ => (v, h - u),
        };
        pts(&p.map(map))
    };
    Some(match prst {
        "rect" | "flowChartProcess" => (rect, false),
        "roundRect" => (round_rect(ss * a("adj", 16667.0)), false),
        "flowChartAlternateProcess" => (round_rect(ss / 6.0), false),
        "flowChartTerminator" => (round_rect(h.min(w) / 2.0), false),
        "ellipse" | "flowChartConnector" => (ellipse, false),
        "triangle" | "flowChartExtract" => {
            let tx = w * if prst == "triangle" { a("adj", 50000.0) } else { 0.5 };
            (pts(&[(tx, 0.0), (w, h), (0.0, h)]), false)
        }
        "flowChartMerge" => (pts(&[(0.0, 0.0), (w, 0.0), (w / 2.0, h)]), false),
        "rtTriangle" => (pts(&[(0.0, 0.0), (w, h), (0.0, h)]), false),
        "diamond" | "flowChartDecision" => (pts(&[(w / 2.0, 0.0), (w, h / 2.0), (w / 2.0, h), (0.0, h / 2.0)]), false),
        "parallelogram" | "flowChartInputOutput" => {
            let o = if prst == "parallelogram" { ss * a("adj", 25000.0) } else { w / 5.0 };
            (pts(&[(o, 0.0), (w, 0.0), (w - o, h), (0.0, h)]), false)
        }
        "trapezoid" | "flowChartManualOperation" => {
            let o = if prst == "trapezoid" { ss * a("adj", 25000.0) } else { w / 5.0 };
            if prst == "trapezoid" { (pts(&[(o, 0.0), (w - o, 0.0), (w, h), (0.0, h)]), false) } else { (pts(&[(0.0, 0.0), (w, 0.0), (w - o, h), (o, h)]), false) }
        }
        "pentagon" | "homePlate" => {
            let o = ss * a("adj", 50000.0);
            (pts(&[(0.0, 0.0), (w - o, 0.0), (w, h / 2.0), (w - o, h), (0.0, h)]), false)
        }
        "chevron" => {
            let o = ss * a("adj", 50000.0);
            (pts(&[(0.0, 0.0), (w - o, 0.0), (w, h / 2.0), (w - o, h), (0.0, h), (o, h / 2.0)]), false)
        }
        "hexagon" | "flowChartPreparation" => {
            let o = if prst == "hexagon" { ss * a("adj", 25000.0) } else { w / 5.0 };
            (pts(&[(o, 0.0), (w - o, 0.0), (w, h / 2.0), (w - o, h), (o, h), (0.0, h / 2.0)]), false)
        }
        "octagon" => {
            let o = ss * a("adj", 29289.0);
            (pts(&[(o, 0.0), (w - o, 0.0), (w, o), (w, h - o), (w - o, h), (o, h), (0.0, h - o), (0.0, o)]), false)
        }
        "plus" | "mathPlus" => {
            let o = ss * a("adj", 25000.0);
            (pts(&[(o, 0.0), (w - o, 0.0), (w - o, o), (w, o), (w, h - o), (w - o, h - o), (w - o, h), (o, h), (o, h - o), (0.0, h - o), (0.0, o), (o, o)]), false)
        }
        "star5" => {
            let (cx, cy) = (w / 2.0, h / 2.0);
            let mut p = Vec::new();
            for i in 0..10 {
                let r = if i % 2 == 0 { 1.0 } else { 0.38 };
                let ang = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::PI / 5.0;
                p.push((cx + cx * r * ang.cos(), cy + cy * r * ang.sin()));
            }
            (pts(&p), false)
        }
        "rightArrow" => (arrow('r'), false),
        "leftArrow" => (arrow('l'), false),
        "downArrow" => (arrow('d'), false),
        "upArrow" => (arrow('u'), false),
        "leftRightArrow" => {
            let th = h * a("adj1", 50000.0);
            let head = (ss * a("adj2", 50000.0)).min(w / 2.0);
            let (s0, s1) = ((h - th) / 2.0, (h + th) / 2.0);
            (pts(&[(0.0, h / 2.0), (head, 0.0), (head, s0), (w - head, s0), (w - head, 0.0), (w, h / 2.0), (w - head, h), (w - head, s1), (head, s1), (head, h)]), false)
        }
        "upDownArrow" => {
            let th = w * a("adj1", 50000.0);
            let head = (ss * a("adj2", 50000.0)).min(h / 2.0);
            let (s0, s1) = ((w - th) / 2.0, (w + th) / 2.0);
            (pts(&[(w / 2.0, 0.0), (w, head), (s1, head), (s1, h - head), (w, h - head), (w / 2.0, h), (0.0, h - head), (s0, h - head), (s0, head), (0.0, head)]), false)
        }
        "wedgeRectCallout" => (box_callout(0.0), false),
        "wedgeRoundRectCallout" => (box_callout(ss * a("adj3", 16667.0)), false),
        "wedgeEllipseCallout" | "cloudCallout" => (ellipse_callout(), false),
        "cloud" => (ellipse, false),
        "flowChartDocument" => (
            format!(
                "M0,0 L{w},0 L{w},{y} C{c1},{y2} {c2},{y3} 0,{y} Z",
                w = n(w),
                y = n(h * 0.83),
                c1 = n(w * 0.6),
                y2 = n(h * 0.65),
                c2 = n(w * 0.4),
                y3 = n(h * 1.05)
            ),
            false,
        ),
        "flowChartPredefinedProcess" => (format!("{rect} M{x1},0 L{x1},{h} M{x2},0 L{x2},{h}", x1 = n(w / 8.0), x2 = n(w * 7.0 / 8.0), h = n(h)), false),
        "can" | "flowChartMagneticDisk" => {
            let ry = ss * a("adj", 25000.0) / 2.0;
            (
                format!(
                    "M0,{ry} A{rx},{ry} 0 0 1 {w},{ry} L{w},{y1} A{rx},{ry} 0 0 1 0,{y1} Z M0,{ry} A{rx},{ry} 0 0 0 {w},{ry}",
                    ry = n(ry),
                    rx = n(w / 2.0),
                    w = n(w),
                    y1 = n(h - ry)
                ),
                false,
            )
        }
        "frame" => {
            let t = ss * a("adj1", 12500.0);
            (format!("{rect} {}", pts(&[(t, t), (t, h - t), (w - t, h - t), (w - t, t)])), false)
        }
        "leftBracket" | "rightBracket" => {
            let r = (h * a("adj", 8333.0)).min(w);
            let d = if prst == "leftBracket" {
                format!("M{w},0 A{w},{r} 0 0 0 0,{r} L0,{y1} A{w},{r} 0 0 0 {w},{h}", w = n(w), r = n(r), y1 = n(h - r), h = n(h))
            } else {
                format!("M0,0 A{w},{r} 0 0 1 {w},{r} L{w},{y1} A{w},{r} 0 0 1 0,{h}", w = n(w), r = n(r), y1 = n(h - r), h = n(h))
            };
            (d, true)
        }
        "leftBrace" | "rightBrace" => {
            let (x0, x1) = if prst == "leftBrace" { (w, 0.0) } else { (0.0, w) };
            let xm = (x0 + x1) / 2.0;
            let r = (h * a("adj1", 8333.0)).min(h / 4.0);
            let d = format!(
                "M{x0},0 Q{xm},0 {xm},{r} L{xm},{a1} Q{xm},{m} {x1},{m} Q{xm},{m} {xm},{a2} L{xm},{b1} Q{xm},{h} {x0},{h}",
                x0 = n(x0),
                x1 = n(x1),
                xm = n(xm),
                r = n(r),
                a1 = n(h / 2.0 - r),
                m = n(h / 2.0),
                a2 = n(h / 2.0 + r),
                b1 = n(h - r),
                h = n(h)
            );
            (d, true)
        }
        "line" | "straightConnector1" => (format!("M0,0 L{},{}", n(w), n(h)), true),
        "bentConnector2" => (format!("M0,0 L{},0 L{},{}", n(w), n(w), n(h)), true),
        "bentConnector3" => {
            let x = w * a("adj1", 50000.0);
            (format!("M0,0 L{x},0 L{x},{h} L{w},{h}", x = n(x), h = n(h), w = n(w)), true)
        }
        "bentConnector4" | "bentConnector5" => {
            let x = w * a("adj1", 50000.0);
            (format!("M0,0 L{x},0 L{x},{h} L{w},{h}", x = n(x), h = n(h), w = n(w)), true)
        }
        "curvedConnector2" | "curvedConnector3" | "curvedConnector4" | "curvedConnector5" => {
            (format!("M0,0 C{},0 {},{} {},{}", n(w / 2.0), n(w / 2.0), n(h), n(w), n(h)), true)
        }
        _ => return None,
    })
}

/// The SVG path of a freeform (custGeom) in a w × h box.
fn custom_path(geom: &Node, w: f64, h: f64) -> (String, bool, bool) {
    let mut d = String::new();
    let (mut filled, mut stroked) = (false, false);
    let Some(list) = geom.child("pathLst") else { return (d, false, false) };
    for p in list.all("path") {
        let pw = p.attr("w").and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0);
        let ph = p.attr("h").and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0);
        let (kx, ky) = (pw.map(|v| w / v).unwrap_or(1.0 / EMU_PT), ph.map(|v| h / v).unwrap_or(1.0 / EMU_PT));
        filled |= p.attr("fill") != Some("none");
        stroked |= p.attr("stroke") != Some("0") && p.attr("stroke") != Some("false");
        let pt = |n0: &Node| -> Option<String> {
            let x = n0.attr("x")?.parse::<f64>().ok()? * kx;
            let y = n0.attr("y")?.parse::<f64>().ok()? * ky;
            Some(format!("{},{}", n(x), n(y)))
        };
        for c in &p.children {
            let ps: Vec<String> = c.all("pt").filter_map(pt).collect();
            match c.name.as_str() {
                "moveTo" if !ps.is_empty() => {
                    let _ = write!(d, "M{} ", ps[0]);
                }
                "lnTo" if !ps.is_empty() => {
                    let _ = write!(d, "L{} ", ps[0]);
                }
                "cubicBezTo" if ps.len() == 3 => {
                    let _ = write!(d, "C{} {} {} ", ps[0], ps[1], ps[2]);
                }
                "quadBezTo" if ps.len() == 2 => {
                    let _ = write!(d, "Q{} {} ", ps[0], ps[1]);
                }
                "close" => d.push_str("Z "),
                _ => {}
            }
        }
    }
    (d, filled, stroked)
}

/// An arrowhead of a line end (headEnd / tailEnd), as an SVG marker.
fn marker(end: Option<&Node>, color: &str, id: &str, width: f64) -> Option<String> {
    let e = end?;
    let kind = e.attr("type")?;
    let size = |k: &str| match e.attr(k) {
        Some("sm") => 2.0,
        Some("lg") => 5.0,
        _ => 3.0,
    };
    // As Excel: in proportion to the line, but not tiny on thin lines.
    let unit = width.max(2.0);
    let (mw, mh) = (size("w") * unit, size("len") * unit);
    let shape = match kind {
        "triangle" => "<path d=\"M0,0 L10,5 L0,10 Z\"/>".to_string(),
        "stealth" => "<path d=\"M0,0 L10,5 L0,10 L3,5 Z\"/>".to_string(),
        "arrow" => format!("<path d=\"M0,0 L10,5 L0,10\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1.5\"/>"),
        "diamond" => "<path d=\"M0,5 L5,0 L10,5 L5,10 Z\"/>".to_string(),
        "oval" => "<circle cx=\"5\" cy=\"5\" r=\"5\"/>".to_string(),
        _ => return None,
    };
    let refx = if matches!(kind, "diamond" | "oval") { 5 } else { 9 };
    Some(format!(
        "<marker id=\"{id}\" viewBox=\"0 0 10 10\" refX=\"{refx}\" refY=\"5\" markerWidth=\"{}\" markerHeight=\"{}\" orient=\"auto-start-reverse\" markerUnits=\"userSpaceOnUse\" fill=\"{color}\" overflow=\"visible\">{shape}</marker>",
        n(mh),
        n(mw)
    ))
}

fn dash(ln: Option<&Node>, width: f64) -> String {
    let Some(d) = ln.and_then(|l| l.child("prstDash")).and_then(|p| p.attr("val")) else { return String::new() };
    let k = width.max(0.5);
    let pattern: &[f64] = match d {
        "dash" => &[4.0, 3.0],
        "sysDash" => &[3.0, 1.0],
        "dot" | "sysDot" => &[1.0, 1.0],
        "lgDash" => &[8.0, 3.0],
        "dashDot" | "sysDashDot" => &[4.0, 3.0, 1.0, 3.0],
        "lgDashDot" => &[8.0, 3.0, 1.0, 3.0],
        "lgDashDotDot" | "sysDashDotDot" => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        _ => return String::new(),
    };
    let v: Vec<String> = pattern.iter().map(|x| n(x * k)).collect();
    format!(" stroke-dasharray=\"{}\"", v.join(" "))
}

fn shape_html(ctx: &Ctx, sp: &Node, x: &Xfrm, w: f64, h: f64) -> String {
    let props = sp.child("spPr");
    let geom_preset = props.and_then(|p| p.child("prstGeom"));
    let geom_custom = props.and_then(|p| p.child("custGeom"));
    let (mut d, mut line) = (String::new(), false);
    let (mut can_fill, mut can_stroke) = (true, true);
    if let Some(g) = geom_preset {
        let prst = g.attr("prst").unwrap_or("rect");
        match preset_path(prst, w, h, &adjusts(g)) {
            Some((p, l)) => (d, line) = (p, l),
            None => d = preset_path("rect", w, h, &HashMap::new()).unwrap().0,
        }
    } else if let Some(g) = geom_custom {
        (d, can_fill, can_stroke) = custom_path(g, w, h);
    } else if sp.name == "cxnSp" {
        d = format!("M0,0 L{},{}", n(w), n(h));
        line = true;
    }
    let ln = props.and_then(|p| p.child("ln"));
    // Fill: the shape's own, else the theme's by its style; lines have none.
    let fill = if line || !can_fill {
        ("none".to_string(), 1.0)
    } else {
        paint(props, &ctx.theme).or_else(|| style_ref(sp, "fillRef", &ctx.theme).map(|(c, _)| (c, 1.0))).unwrap_or(("none".into(), 1.0))
    };
    let stroke = if !can_stroke {
        ("none".to_string(), 1.0)
    } else {
        paint(ln, &ctx.theme).or_else(|| style_ref(sp, "lnRef", &ctx.theme).map(|(c, _)| (c, 1.0))).unwrap_or(("none".into(), 1.0))
    };
    let width = ln.and_then(|l| l.attr("w")).and_then(|v| v.parse::<f64>().ok()).map(|v| v / EMU_PT).unwrap_or_else(|| {
        match style_ref(sp, "lnRef", &ctx.theme).map(|(_, i)| i) {
            Some(1) => 0.5,
            Some(3) => 1.5,
            Some(_) => 1.0,
            None => 0.75,
        }
    });
    let mut svg = String::new();
    if !d.is_empty() && (fill.0 != "none" || stroke.0 != "none") {
        let id = ctx.ids.get();
        ctx.ids.set(id + 1);
        let mut defs = String::new();
        let mut ends = String::new();
        if stroke.0 != "none" {
            for (k, attr) in [("headEnd", "marker-start"), ("tailEnd", "marker-end")] {
                let mid = format!("xm{id}{k}");
                if let Some(m) = marker(ln.and_then(|l| l.child(k)), &stroke.0, &mid, width) {
                    defs.push_str(&m);
                    let _ = write!(ends, " {attr}=\"url(#{mid})\"");
                }
            }
        }
        let flip = if x.flip_h || x.flip_v {
            format!(" transform=\"translate({},{}) scale({},{})\"", if x.flip_h { n(w) } else { "0".into() }, if x.flip_v { n(h) } else { "0".into() }, if x.flip_h { -1 } else { 1 }, if x.flip_v { -1 } else { 1 })
        } else {
            String::new()
        };
        let op = |name: &str, a: f64| if a < 1.0 { format!(" {name}-opacity=\"{}\"", n(a)) } else { String::new() };
        let _ = write!(
            svg,
            "<svg viewBox=\"0 0 {w} {h}\" preserveAspectRatio=\"none\" style=\"position:absolute;inset:0;width:100%;height:100%;overflow:visible\">{}<path d=\"{d}\" fill=\"{}\"{} stroke=\"{}\" stroke-width=\"{}\"{}{} stroke-linejoin=\"round\"{flip}{ends}/></svg>",
            if defs.is_empty() { String::new() } else { format!("<defs>{defs}</defs>") },
            fill.0,
            op("fill", fill.1),
            stroke.0,
            n(width),
            op("stroke", stroke.1),
            dash(ln, width),
            w = n(w.max(0.01)),
            h = n(h.max(0.01)),
        );
    }
    let text = sp.child("txBody").map(|t| text_html(ctx, sp, t)).unwrap_or_default();
    format!("{svg}{text}")
}

/// The text of a shape (txBody).
fn text_html(ctx: &Ctx, sp: &Node, tx: &Node) -> String {
    let body = tx.child("bodyPr");
    let ins = |k: &str, d: f64| body.and_then(|b| b.attr(k)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(d) / EMU_PT;
    let (l, t, r, b) = (ins("lIns", 91440.0), ins("tIns", 45720.0), ins("rIns", 91440.0), ins("bIns", 45720.0));
    let anchor = match body.and_then(|b| b.attr("anchor")) {
        Some("ctr") => "center",
        Some("b") => "flex-end",
        _ => "flex-start",
    };
    let vert = matches!(body.and_then(|b| b.attr("vert")), Some("vert" | "eaVert" | "wordArtVert" | "mongolianVert"));
    let nowrap = body.and_then(|b| b.attr("wrap")) == Some("none");
    let base_color = style_ref(sp, "fontRef", &ctx.theme).map(|(c, _)| c).unwrap_or_else(|| "#000000".into());
    // Defaults from the list style (lvl1pPr/defRPr).
    let lst = tx.child("lstStyle").and_then(|l| l.child("lvl1pPr"));
    let mut paras = String::new();
    let mut any = false;
    for p in tx.all("p") {
        let ppr = p.child("pPr");
        let algn = ppr.and_then(|x| x.attr("algn")).or_else(|| lst.and_then(|x| x.attr("algn")));
        let align = match algn {
            Some("ctr") => "center",
            Some("r") => "right",
            Some("just") | Some("dist") => "justify",
            _ => "left",
        };
        let def = ppr.and_then(|x| x.child("defRPr")).or_else(|| lst.and_then(|x| x.child("defRPr")));
        let mut runs = String::new();
        if let Some(bu) = ppr.and_then(|x| x.child("buChar")).and_then(|c| c.attr("char")) {
            let _ = write!(runs, "{}&nbsp;", esc(bu));
        }
        let mut size_last = def.and_then(|d| d.attr("sz")).and_then(|v| v.parse::<f64>().ok()).map(|v| v / 100.0);
        for c in &p.children {
            match c.name.as_str() {
                "r" | "fld" => {
                    let s = c.child("t").map(|t| t.text.clone()).unwrap_or_default();
                    if s.is_empty() {
                        continue;
                    }
                    any = true;
                    let rpr = c.child("rPr");
                    let css = run_css(ctx, rpr, def, &base_color);
                    if let Some(sz) = rpr.and_then(|x| x.attr("sz")).and_then(|v| v.parse::<f64>().ok()) {
                        size_last = Some(sz / 100.0);
                    }
                    let _ = write!(runs, "<span style=\"{css}\">{}</span>", esc(&s));
                }
                "br" => runs.push_str("<br>"),
                _ => {}
            }
        }
        if runs.is_empty() {
            // An empty paragraph keeps its line.
            let sz = p.child("endParaRPr").and_then(|x| x.attr("sz")).and_then(|v| v.parse::<f64>().ok()).map(|v| v / 100.0).or(size_last).unwrap_or(11.0);
            runs = format!("<span style=\"font-size:{}pt\">&nbsp;</span>", n(sz));
        }
        let _ = write!(paras, "<p style=\"margin:0;text-align:{align}\">{runs}</p>");
    }
    if !any {
        return String::new();
    }
    let mut style = format!(
        "position:absolute;inset:0;box-sizing:border-box;padding:{}pt {}pt {}pt {}pt;display:flex;flex-direction:column;justify-content:{anchor};overflow:visible;line-height:1.2;font-family:{};font-size:11pt;color:{base_color};",
        n(t),
        n(r),
        n(b),
        n(l),
        esc(&font_family(&ctx.font.0))
    );
    style.push_str(if nowrap { "white-space:pre;" } else { "white-space:pre-wrap;overflow-wrap:anywhere;" });
    if vert {
        style.push_str("writing-mode:vertical-rl;");
    }
    format!("<div style=\"{style}\">{paras}</div>")
}

fn run_css(ctx: &Ctx, rpr: Option<&Node>, def: Option<&Node>, base_color: &str) -> String {
    let get = |k: &str| rpr.and_then(|x| x.attr(k)).or_else(|| def.and_then(|x| x.attr(k)));
    let mut css = String::new();
    if let Some(sz) = get("sz").and_then(|v| v.parse::<f64>().ok()) {
        let _ = write!(css, "font-size:{}pt;", n(sz / 100.0));
    }
    if matches!(get("b"), Some("1" | "true")) {
        css.push_str("font-weight:bold;");
    }
    if matches!(get("i"), Some("1" | "true")) {
        css.push_str("font-style:italic;");
    }
    let mut deco = Vec::new();
    if get("u").is_some_and(|u| u != "none") {
        deco.push("underline");
    }
    if get("strike").is_some_and(|s| s != "noStrike") {
        deco.push("line-through");
    }
    if !deco.is_empty() {
        let _ = write!(css, "text-decoration:{};", deco.join(" "));
    }
    let face = |n0: Option<&Node>| -> Option<String> {
        let n0 = n0?;
        for k in ["ea", "latin"] {
            if let Some(f) = n0.child(k).and_then(|c| c.attr("typeface"))
                && !f.is_empty()
                && !f.starts_with('+')
            {
                return Some(f.to_string());
            }
        }
        None
    };
    if let Some(f) = face(rpr).or_else(|| face(def)) {
        let _ = write!(css, "font-family:{};", esc(&font_family(&f)));
    }
    let color = rpr.and_then(|x| x.child("solidFill")).or_else(|| def.and_then(|x| x.child("solidFill"))).and_then(|f| color_of(f, &ctx.theme));
    if let Some(c) = color
        && c != base_color
    {
        let _ = write!(css, "color:{c};");
    }
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets() {
        let none = HashMap::new();
        assert_eq!(preset_path("rect", 10.0, 5.0, &none).unwrap().0, "M0,0 L10,0 L10,5 L0,5 Z");
        assert_eq!(preset_path("diamond", 10.0, 6.0, &none).unwrap().0, "M5,0 L10,3 L5,6 L0,3 Z");
        // A right arrow: shaft half the height, head half the short side.
        assert_eq!(preset_path("rightArrow", 20.0, 10.0, &none).unwrap().0, "M0,2.5 L15,2.5 L15,0 L20,5 L15,10 L15,7.5 L0,7.5 Z");
        assert!(preset_path("straightConnector1", 10.0, 0.0, &none).unwrap().1, "a line");
        let mut adj = HashMap::new();
        adj.insert("adj".to_string(), 50000.0);
        assert!(preset_path("roundRect", 10.0, 4.0, &adj).unwrap().0.starts_with("M2,0 L8,0 A2,2"));
        assert!(preset_path("heart", 10.0, 10.0, &none).is_none());
        // A callout is one outline: the tail (tip below left) is in the bottom edge.
        let mut tail = HashMap::new();
        tail.insert("adj1".to_string(), -25000.0);
        tail.insert("adj2".to_string(), 100000.0);
        assert_eq!(preset_path("wedgeRectCallout", 120.0, 60.0, &tail).unwrap().0, "M0,0 L120,0 L120,60 L50,60 L30,90 L20,60 L0,60 L0,0 Z");
        assert_eq!(preset_path("wedgeRectCallout", 120.0, 60.0, &none).unwrap().0.matches('M').count(), 1);
    }
}
