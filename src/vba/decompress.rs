/// Expands a CompressedContainer ([MS-OVBA] 2.4.1): a signature byte 1 and
/// chunks of up to 4096 decompressed bytes, each either stored as is or as
/// token sequences (a flag byte, then 8 tokens: literal bytes and copy
/// tokens referring back into the chunk).
pub fn decompress(b: &[u8]) -> Result<Vec<u8>, String> {
    if b.first() != Some(&1) {
        return Err("not a compressed container".into());
    }
    let mut out = Vec::new();
    let mut pos = 1;
    while pos + 2 <= b.len() {
        let h = u16::from_le_bytes([b[pos], b[pos + 1]]) as usize;
        let end = (pos + (h & 0x0FFF) + 3).min(b.len());
        pos += 2;
        let start = out.len();
        if h & 0x8000 == 0 {
            // uncompressed chunk
            let n = 4096.min(b.len() - pos);
            out.extend_from_slice(&b[pos..pos + n]);
            pos += n;
            continue;
        }
        while pos < end {
            let flags = b[pos];
            pos += 1;
            let mut bit = 0;
            while bit < 8 && pos < end {
                if flags & (1 << bit) == 0 {
                    out.push(b[pos]);
                    pos += 1;
                    bit += 1;
                    continue;
                }
                if pos + 2 > end {
                    return Err("truncated copy token".into());
                }
                let tok = u16::from_le_bytes([b[pos], b[pos + 1]]) as usize;
                pos += 2;
                let bits = copy_token_bits(out.len() - start);
                let length = (tok & (0xFFFF >> bits)) + 3;
                let back = (tok >> (16 - bits)) + 1;
                if back > out.len() - start {
                    return Err("copy token out of range".into());
                }
                let src = out.len() - back;
                for i in 0..length {
                    out.push(out[src + i]);
                }
                bit += 1;
            }
        }
        pos = end;
    }
    Ok(out)
}

/// The number of offset bits of a copy token at the given position in the
/// decompressed chunk: ceil(log2(pos)), at least 4.
pub fn copy_token_bits(pos: usize) -> usize {
    let mut bits = 4;
    while (1usize << bits) < pos {
        bits += 1;
    }
    bits
}
