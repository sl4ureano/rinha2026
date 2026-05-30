//! Parsers numéricos compartilhados (linear + scanner).

#[inline]
pub fn parse_u32_bytes(s: &[u8]) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut n = 0u32;
    for &b in s {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n * 10 + (b - b'0') as u32;
    }
    Some(n)
}

#[inline]
pub fn parse_f32_bytes(s: &[u8]) -> Option<f32> {
    if s.is_empty() {
        return None;
    }
    let mut i = 0usize;
    let neg = s[0] == b'-';
    if neg {
        i += 1;
    }
    let mut int_part = 0u64;
    while i < s.len() && s[i].is_ascii_digit() {
        let d = (s[i] - b'0') as u64;
        int_part = int_part.checked_mul(10)?.checked_add(d)?;
        i += 1;
    }
    let mut frac = 0u64;
    let mut frac_div = 1u64;
    if i < s.len() && s[i] == b'.' {
        i += 1;
        while i < s.len() && s[i].is_ascii_digit() {
            let d = (s[i] - b'0') as u64;
            frac = frac.checked_mul(10)?.checked_add(d)?;
            frac_div = frac_div.checked_mul(10)?;
            i += 1;
        }
    }
    if i != s.len() {
        return None;
    }
    let v = (int_part as f64 + frac as f64 / frac_div as f64) as f32;
    Some(if neg { -v } else { v })
}
