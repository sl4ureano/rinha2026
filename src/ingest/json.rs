//! Allocation-free extractor for the 14 fields we care about. Tolerant of
//! whitespace and key reorderings within objects, but assumes well-formed JSON.

#[derive(Debug, Clone, Default)]
pub struct RawPayload<'a> {
    pub amount: f32,
    pub installments: u32,
    pub requested_at: &'a [u8],
    pub customer_avg_amount: f32,
    pub tx_count_24h: u32,
    pub known_merchants: &'a [u8],
    pub merchant_id: &'a [u8],
    pub merchant_mcc: &'a [u8],
    pub merchant_avg_amount: f32,
    pub is_online: bool,
    pub card_present: bool,
    pub km_from_home: f32,
    /// ISO-8601 timestamp slice when `last_transaction` is present; None when null.
    pub last_timestamp: Option<&'a [u8]>,
    pub last_km: Option<f32>,
}

pub fn extract(body: &[u8]) -> Option<RawPayload<'_>> {
    let mut p = RawPayload::default();
    let mut s = Scanner::new(body);
    s.expect(b'{')?;

    while !s.at_end() {
        s.skip_ws();
        if s.peek() == Some(b'}') {
            s.bump();
            break;
        }

        let key = s.read_string()?;
        s.skip_ws();
        s.expect(b':')?;
        s.skip_ws();

        match key {
            b"transaction" => parse_transaction(&mut s, &mut p)?,
            b"customer" => parse_customer(&mut s, &mut p)?,
            b"merchant" => parse_merchant(&mut s, &mut p)?,
            b"terminal" => parse_terminal(&mut s, &mut p)?,
            b"last_transaction" => parse_last_transaction(&mut s, &mut p)?,
            _ => s.skip_value()?,
        }

        s.skip_ws();
        if s.peek() == Some(b',') {
            s.bump();
        }
    }
    Some(p)
}

fn parse_transaction<'a>(s: &mut Scanner<'a>, p: &mut RawPayload<'a>) -> Option<()> {
    s.expect(b'{')?;
    loop {
        s.skip_ws();
        if s.peek() == Some(b'}') {
            s.bump();
            return Some(());
        }
        let k = s.read_string()?;
        s.skip_ws();
        s.expect(b':')?;
        s.skip_ws();
        match k {
            b"amount" => p.amount = s.read_f32()?,
            b"installments" => p.installments = s.read_u32()?,
            b"requested_at" => p.requested_at = s.read_string()?,
            _ => s.skip_value()?,
        }
        s.skip_ws();
        if s.peek() == Some(b',') {
            s.bump();
        }
    }
}

fn parse_customer<'a>(s: &mut Scanner<'a>, p: &mut RawPayload<'a>) -> Option<()> {
    s.expect(b'{')?;
    loop {
        s.skip_ws();
        if s.peek() == Some(b'}') {
            s.bump();
            return Some(());
        }
        let k = s.read_string()?;
        s.skip_ws();
        s.expect(b':')?;
        s.skip_ws();
        match k {
            b"avg_amount" => p.customer_avg_amount = s.read_f32()?,
            b"tx_count_24h" => p.tx_count_24h = s.read_u32()?,
            b"known_merchants" => p.known_merchants = s.read_array_raw()?,
            _ => s.skip_value()?,
        }
        s.skip_ws();
        if s.peek() == Some(b',') {
            s.bump();
        }
    }
}

fn parse_merchant<'a>(s: &mut Scanner<'a>, p: &mut RawPayload<'a>) -> Option<()> {
    s.expect(b'{')?;
    loop {
        s.skip_ws();
        if s.peek() == Some(b'}') {
            s.bump();
            return Some(());
        }
        let k = s.read_string()?;
        s.skip_ws();
        s.expect(b':')?;
        s.skip_ws();
        match k {
            b"id" => p.merchant_id = s.read_string()?,
            b"mcc" => p.merchant_mcc = s.read_string()?,
            b"avg_amount" => p.merchant_avg_amount = s.read_f32()?,
            _ => s.skip_value()?,
        }
        s.skip_ws();
        if s.peek() == Some(b',') {
            s.bump();
        }
    }
}

fn parse_terminal<'a>(s: &mut Scanner<'a>, p: &mut RawPayload<'a>) -> Option<()> {
    s.expect(b'{')?;
    loop {
        s.skip_ws();
        if s.peek() == Some(b'}') {
            s.bump();
            return Some(());
        }
        let k = s.read_string()?;
        s.skip_ws();
        s.expect(b':')?;
        s.skip_ws();
        match k {
            b"is_online" => p.is_online = s.read_bool()?,
            b"card_present" => p.card_present = s.read_bool()?,
            b"km_from_home" => p.km_from_home = s.read_f32()?,
            _ => s.skip_value()?,
        }
        s.skip_ws();
        if s.peek() == Some(b',') {
            s.bump();
        }
    }
}

fn parse_last_transaction<'a>(s: &mut Scanner<'a>, p: &mut RawPayload<'a>) -> Option<()> {
    s.skip_ws();
    if s.peek_word(b"null") {
        s.advance(4);
        return Some(());
    }
    s.expect(b'{')?;
    loop {
        s.skip_ws();
        if s.peek() == Some(b'}') {
            s.bump();
            break;
        }
        let k = s.read_string()?;
        s.skip_ws();
        s.expect(b':')?;
        s.skip_ws();
        match k {
            b"timestamp" => p.last_timestamp = Some(s.read_string()?),
            b"km_from_current" => p.last_km = Some(s.read_f32()?),
            _ => s.skip_value()?,
        }
        s.skip_ws();
        if s.peek() == Some(b',') {
            s.bump();
        }
    }
    Some(())
}

#[inline]
fn parse_u32_bytes(s: &[u8]) -> Option<u32> {
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
fn parse_f32_bytes(s: &[u8]) -> Option<f32> {
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

struct Scanner<'a> {
    buf: &'a [u8],
    i: usize,
}

impl<'a> Scanner<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, i: 0 }
    }
    #[inline]
    fn at_end(&self) -> bool {
        self.i >= self.buf.len()
    }
    #[inline]
    fn peek(&self) -> Option<u8> {
        self.buf.get(self.i).copied()
    }
    #[inline]
    fn bump(&mut self) {
        self.i += 1;
    }
    #[inline]
    fn advance(&mut self, n: usize) {
        self.i += n;
    }

    fn expect(&mut self, c: u8) -> Option<()> {
        if self.peek()? == c {
            self.bump();
            Some(())
        } else {
            None
        }
    }
    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                self.bump();
            } else {
                break;
            }
        }
    }
    fn peek_word(&self, w: &[u8]) -> bool {
        self.buf.get(self.i..self.i + w.len()) == Some(w)
    }
    fn read_string(&mut self) -> Option<&'a [u8]> {
        self.expect(b'"')?;
        let start = self.i;
        while let Some(c) = self.peek() {
            if c == b'"' {
                let s = &self.buf[start..self.i];
                self.bump();
                return Some(s);
            }
            if c == b'\\' {
                self.bump();
            }
            self.bump();
        }
        None
    }
    fn read_f32(&mut self) -> Option<f32> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if matches!(c, b'-' | b'+' | b'.' | b'0'..=b'9' | b'e' | b'E') {
                self.bump();
            } else {
                break;
            }
        }
        let slice = &self.buf[start..self.i];
        if let Some(v) = parse_f32_bytes(slice) {
            return Some(v);
        }
        // Scientific notation only — k6 payloads are plain decimals.
        std::str::from_utf8(slice).ok()?.parse().ok()
    }
    fn read_u32(&mut self) -> Option<u32> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.bump();
            } else {
                break;
            }
        }
        parse_u32_bytes(&self.buf[start..self.i])
    }
    fn read_bool(&mut self) -> Option<bool> {
        if self.peek_word(b"true") {
            self.advance(4);
            Some(true)
        } else if self.peek_word(b"false") {
            self.advance(5);
            Some(false)
        } else {
            None
        }
    }
    fn read_array_raw(&mut self) -> Option<&'a [u8]> {
        self.expect(b'[')?;
        let start = self.i;
        let mut depth = 1;
        let mut in_str = false;
        while let Some(c) = self.peek() {
            if in_str {
                if c == b'\\' {
                    self.bump();
                } else if c == b'"' {
                    in_str = false;
                }
                self.bump();
                continue;
            }
            match c {
                b'"' => {
                    in_str = true;
                    self.bump();
                }
                b'[' => {
                    depth += 1;
                    self.bump();
                }
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        let s = &self.buf[start..self.i];
                        self.bump();
                        return Some(s);
                    }
                    self.bump();
                }
                _ => {
                    self.bump();
                }
            }
        }
        None
    }
    fn skip_value(&mut self) -> Option<()> {
        self.skip_ws();
        match self.peek()? {
            b'"' => {
                self.read_string()?;
                Some(())
            }
            b'{' => self.skip_object(),
            b'[' => {
                self.read_array_raw()?;
                Some(())
            }
            b't' | b'f' => {
                self.read_bool()?;
                Some(())
            }
            b'n' => {
                if self.peek_word(b"null") {
                    self.advance(4);
                    Some(())
                } else {
                    None
                }
            }
            _ => {
                let _ = self.read_f32()?;
                Some(())
            }
        }
    }
    fn skip_object(&mut self) -> Option<()> {
        self.expect(b'{')?;
        let mut depth = 1;
        let mut in_str = false;
        while let Some(c) = self.peek() {
            if in_str {
                if c == b'\\' {
                    self.bump();
                } else if c == b'"' {
                    in_str = false;
                }
                self.bump();
                continue;
            }
            match c {
                b'"' => {
                    in_str = true;
                    self.bump();
                }
                b'{' => {
                    depth += 1;
                    self.bump();
                }
                b'}' => {
                    depth -= 1;
                    self.bump();
                    if depth == 0 {
                        return Some(());
                    }
                }
                _ => {
                    self.bump();
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"{"id":"tx-100","transaction":{"amount":41.12,"installments":2,"requested_at":"2026-03-11T18:45:53Z"},"customer":{"avg_amount":82.24,"tx_count_24h":3,"known_merchants":["MERC-003","MERC-016"]},"merchant":{"id":"MERC-016","mcc":"5411","avg_amount":60.25},"terminal":{"is_online":false,"card_present":true,"km_from_home":29.2331},"last_transaction":null}"#;

    #[test]
    fn extracts_known_fields() {
        let p = extract(SAMPLE).unwrap();
        assert!((p.amount - 41.12).abs() < 0.001);
        assert_eq!(p.installments, 2);
        assert_eq!(p.merchant_id, b"MERC-016");
        assert_eq!(p.merchant_mcc, b"5411");
        assert!(!p.is_online);
        assert!(p.card_present);
        assert!(p.last_timestamp.is_none());
        assert!(p.last_km.is_none());
    }
}
