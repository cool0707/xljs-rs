const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The image in an .ico file (or a PNG image) that best fits size pixels:
/// the smallest one at least that large, else the largest. The result is a
/// PNG or a DIB, as Windows' CreateIconFromResourceEx takes. None when data
/// is neither an .ico file nor a PNG image.
pub fn icon_image(data: &[u8], size: usize) -> Option<&[u8]> {
    if data.starts_with(PNG_MAGIC) {
        return Some(data);
    }
    let u16at = |i: usize| u16::from_le_bytes([data[i], data[i + 1]]) as usize;
    let u32at = |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().unwrap()) as usize;
    // ICONDIR: reserved 0, type 1, count; then 16-byte entries.
    if data.len() < 6 || u16at(0) != 0 || u16at(2) != 1 {
        return None;
    }
    let count = u16at(4);
    if count == 0 || data.len() < 6 + 16 * count {
        return None;
    }
    let mut best: Option<&[u8]> = None;
    let (mut best_w, mut best_bpp) = (0, 0);
    for i in 0..count {
        let e = 6 + 16 * i;
        let w = if data[e] == 0 { 256 } else { data[e] as usize };
        let bpp = u16at(e + 6);
        let n = u32at(e + 8);
        let off = u32at(e + 12);
        if n == 0 || off.checked_add(n).is_none_or(|end| end > data.len()) {
            continue;
        }
        let better = match best {
            None => true,
            Some(_) if (w >= size) != (best_w >= size) => w >= size, // big enough wins
            Some(_) if w != best_w => (w >= size) == (w < best_w),   // smallest big enough, or largest
            Some(_) => bpp > best_bpp,
        };
        if better {
            best = Some(&data[off..off + n]);
            best_w = w;
            best_bpp = bpp;
        }
    }
    best
}

/// Whether data can be used as a window icon.
pub fn check_icon(data: &[u8]) -> bool {
    icon_image(data, 32).is_some()
}
