//! A strict JSON reader that preserves what `serde_json` discards.
//!
//! `DECONTAM-001` requires rejecting duplicate JSON keys, and `serde_json`
//! silently keeps the last one. It also requires emitting RFC 8785 canonical
//! bytes, which needs the *raw* text of every number: `serde_json` parses an
//! integer too large for `i64`/`u64` straight into `f64`, so the evidence that a
//! value was not exactly representable is destroyed before it can be detected.
//! Both requirements point at the same answer -- parse it here, keep everything,
//! and fail closed rather than guess.

use crate::error::{ProductError, Result};
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// The number exactly as it appeared. Interpretation is deferred to
    /// canonicalization, which is the only place that has to decide whether the
    /// value is representable.
    Number(String),
    String(String),
    Array(Vec<Json>),
    /// Insertion-ordered, because duplicate detection has already happened and
    /// canonical output re-sorts anyway.
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(value) => Some(value),
            _ => None,
        }
    }
}

fn invalid(message: impl Into<String>) -> ProductError {
    ProductError::integrity("BENCHMARK_JSON_INVALID", message)
}

pub struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    /// Parse exactly one JSON value from `bytes`, rejecting trailing content.
    pub fn parse_one(bytes: &'a [u8]) -> Result<Json> {
        let mut parser = Parser { bytes, position: 0 };
        parser.skip_whitespace();
        let value = parser.value(0)?;
        parser.skip_whitespace();
        if parser.position != parser.bytes.len() {
            return Err(invalid("trailing content after a JSON value"));
        }
        Ok(value)
    }

    fn peek(&self) -> Result<u8> {
        self.bytes
            .get(self.position)
            .copied()
            .ok_or_else(|| invalid("unexpected end of JSON input"))
    }

    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.bytes.get(self.position) {
            // JSON permits exactly these four.
            if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                self.position += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, byte: u8) -> Result<()> {
        if self.peek()? != byte {
            return Err(invalid(format!(
                "expected '{}' at byte {}",
                byte as char, self.position
            )));
        }
        self.position += 1;
        Ok(())
    }

    fn literal(&mut self, text: &str, value: Json) -> Result<Json> {
        if self.bytes[self.position..].starts_with(text.as_bytes()) {
            self.position += text.len();
            return Ok(value);
        }
        Err(invalid("unrecognized JSON literal"))
    }

    fn value(&mut self, depth: usize) -> Result<Json> {
        // Bounded so a hostile document cannot exhaust the stack.
        if depth > 128 {
            return Err(invalid("JSON nesting exceeded its bound"));
        }
        match self.peek()? {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => Ok(Json::String(self.string()?)),
            b't' => self.literal("true", Json::Bool(true)),
            b'f' => self.literal("false", Json::Bool(false)),
            b'n' => self.literal("null", Json::Null),
            // Python's `json` module emits bare `NaN` and `Infinity`, which RFC
            // 8259 forbids, and the EvalPlus assets were produced with it. They
            // are recognized explicitly rather than tolerated loosely: they parse
            // into the same `Number` shape, and canonicalization then refuses
            // them, because RFC 8785 has no representation for a non-finite value
            // either. Accepting the token and refusing the canonical form is what
            // keeps the refusal visible instead of turning into a silent zero.
            b'N' => self.literal("NaN", Json::Number("NaN".to_owned())),
            b'I' => self.literal("Infinity", Json::Number("Infinity".to_owned())),
            b'-' | b'0'..=b'9' => self.number(),
            other => Err(invalid(format!(
                "unexpected byte '{}' at {}",
                other as char, self.position
            ))),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json> {
        self.expect(b'{')?;
        let mut entries: Vec<(String, Json)> = Vec::new();
        self.skip_whitespace();
        if self.peek()? == b'}' {
            self.position += 1;
            return Ok(Json::Object(entries));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            // The requirement that motivated writing this parser.
            if entries.iter().any(|(name, _)| name == &key) {
                return Err(invalid(format!("duplicate JSON key {key:?}")));
            }
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            entries.push((key, value));
            self.skip_whitespace();
            match self.peek()? {
                b',' => self.position += 1,
                b'}' => {
                    self.position += 1;
                    return Ok(Json::Object(entries));
                }
                _ => return Err(invalid("expected ',' or '}' in a JSON object")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek()? == b']' {
            self.position += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek()? {
                b',' => self.position += 1,
                b']' => {
                    self.position += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(invalid("expected ',' or ']' in a JSON array")),
            }
        }
    }

    fn string(&mut self) -> Result<String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let byte = self.peek()?;
            match byte {
                b'"' => {
                    self.position += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.position += 1;
                    let escape = self.peek()?;
                    self.position += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(invalid("unrecognized JSON string escape")),
                    }
                }
                // Raw control characters are not permitted in JSON strings.
                0x00..=0x1f => return Err(invalid("a control character appears in a JSON string")),
                _ => {
                    // Decode one UTF-8 scalar; the whole document was validated as
                    // UTF-8 before parsing, so this only needs to find the width.
                    let rest = &self.bytes[self.position..];
                    let text = std::str::from_utf8(rest)
                        .map_err(|_| invalid("invalid UTF-8 in a JSON string"))?;
                    let character = text
                        .chars()
                        .next()
                        .ok_or_else(|| invalid("unexpected end of a JSON string"))?;
                    out.push(character);
                    self.position += character.len_utf8();
                }
            }
        }
    }

    fn unicode_escape(&mut self) -> Result<char> {
        let high = self.hex4()?;
        if !(0xd800..0xdc00).contains(&high) {
            return char::from_u32(u32::from(high))
                .ok_or_else(|| invalid("a \\u escape is not a Unicode scalar"));
        }
        // A high surrogate must be followed by its low surrogate.
        if self.bytes.get(self.position) != Some(&b'\\')
            || self.bytes.get(self.position + 1) != Some(&b'u')
        {
            return Err(invalid(
                "an unpaired high surrogate appears in a JSON string",
            ));
        }
        self.position += 2;
        let low = self.hex4()?;
        if !(0xdc00..0xe000).contains(&low) {
            return Err(invalid(
                "a high surrogate is not followed by a low surrogate",
            ));
        }
        let scalar = 0x10000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(low) - 0xdc00);
        char::from_u32(scalar).ok_or_else(|| invalid("a surrogate pair is not a Unicode scalar"))
    }

    fn hex4(&mut self) -> Result<u16> {
        let slice = self
            .bytes
            .get(self.position..self.position + 4)
            .ok_or_else(|| invalid("a truncated \\u escape"))?;
        let text = std::str::from_utf8(slice).map_err(|_| invalid("a malformed \\u escape"))?;
        if !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid("a \\u escape is not four hex digits"));
        }
        self.position += 4;
        u16::from_str_radix(text, 16).map_err(|_| invalid("a \\u escape is not four hex digits"))
    }

    /// Consume a JSON number, returning its bytes verbatim.
    fn number(&mut self) -> Result<Json> {
        let start = self.position;
        if self.peek()? == b'-' {
            self.position += 1;
            // The negative half of the same Python extension.
            if self.bytes[self.position..].starts_with(b"Infinity") {
                self.position += "Infinity".len();
                return Ok(Json::Number("-Infinity".to_owned()));
            }
        }
        // Integer part: a single zero, or a nonzero digit followed by digits.
        match self.peek()? {
            b'0' => self.position += 1,
            b'1'..=b'9' => {
                while matches!(self.bytes.get(self.position), Some(b'0'..=b'9')) {
                    self.position += 1;
                }
            }
            _ => return Err(invalid("a JSON number has no integer part")),
        }
        if self.bytes.get(self.position) == Some(&b'.') {
            self.position += 1;
            if !matches!(self.bytes.get(self.position), Some(b'0'..=b'9')) {
                return Err(invalid("a JSON number has an empty fraction"));
            }
            while matches!(self.bytes.get(self.position), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }
        if matches!(self.bytes.get(self.position), Some(b'e' | b'E')) {
            self.position += 1;
            if matches!(self.bytes.get(self.position), Some(b'+' | b'-')) {
                self.position += 1;
            }
            if !matches!(self.bytes.get(self.position), Some(b'0'..=b'9')) {
                return Err(invalid("a JSON number has an empty exponent"));
            }
            while matches!(self.bytes.get(self.position), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.position])
            .map_err(|_| invalid("a JSON number is not ASCII"))?;
        Ok(Json::Number(text.to_owned()))
    }
}

/// Serialize to RFC 8785 canonical JSON bytes.
///
/// Fails closed on any number the specification cannot represent. RFC 8785
/// serializes numbers through ECMAScript `Number::toString`, i.e. as IEEE-754
/// doubles, so a literal that does not survive a round trip through `f64` has no
/// canonical form. Emitting the nearest double anyway would silently rewrite the
/// value -- a 55-digit integer would become `6.775685320645824e+54` -- and the
/// result is used for exact byte matching, so a silent rewrite is worse than a
/// refusal.
pub fn canonicalize(value: &Json) -> Result<Vec<u8>> {
    let mut out = String::new();
    write_canonical(value, &mut out)?;
    Ok(out.into_bytes())
}

fn write_canonical(value: &Json, out: &mut String) -> Result<()> {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Number(raw) => out.push_str(&canonical_number(raw)?),
        Json::String(text) => write_canonical_string(text, out),
        Json::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out)?;
            }
            out.push(']');
        }
        Json::Object(entries) => {
            // RFC 8785 orders members by their UTF-16 code units.
            let mut ordered: Vec<&(String, Json)> = entries.iter().collect();
            ordered.sort_by(|left, right| {
                let left_units: Vec<u16> = left.0.encode_utf16().collect();
                let right_units: Vec<u16> = right.0.encode_utf16().collect();
                left_units.cmp(&right_units)
            });
            out.push('{');
            for (index, (key, item)) in ordered.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical_string(key, out);
                out.push(':');
                write_canonical(item, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_canonical_string(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// The ECMAScript `Number::toString` form of a JSON number literal.
fn canonical_number(raw: &str) -> Result<String> {
    let value: f64 = raw
        .parse()
        .map_err(|_| invalid("a JSON number is unparsable"))?;
    if !value.is_finite() {
        return Err(ProductError::integrity(
            "BENCHMARK_NUMBER_NOT_REPRESENTABLE",
            format!("the JSON number {raw} is not finite as an IEEE-754 double"),
        ));
    }
    // Round-trip check: the canonical form is defined on the double, so a literal
    // that is not exactly a double has no canonical form and must not be guessed.
    if !round_trips(raw, value) {
        return Err(ProductError::integrity(
            "BENCHMARK_NUMBER_NOT_REPRESENTABLE",
            format!("the JSON number {raw} is not exactly representable as an IEEE-754 double"),
        ));
    }
    Ok(ecmascript_number_to_string(value))
}

/// Whether `raw` denotes exactly `value`, comparing in the literal's own terms.
///
/// Reformatting the double and comparing strings would reject `1.0` and `1e2`,
/// which are exact. Comparing the shortest round-trip form of the parsed double
/// against a re-parse of the literal is what actually answers the question.
fn round_trips(raw: &str, value: f64) -> bool {
    // A shortest-round-trip printer reproduces the same double, so the only way a
    // literal loses information is if it carries more precision than a double
    // holds. Detect that by printing the double and re-parsing both.
    let printed = format!("{value:e}");
    match printed.parse::<f64>() {
        Ok(reparsed) => {
            if reparsed.to_bits() != value.to_bits() {
                return false;
            }
        }
        Err(_) => return false,
    }
    // Compare significant digits: strip sign, exponent, and leading/trailing
    // zeros from the literal, and require the double's shortest form to carry at
    // least as many significant digits.
    let literal_digits = significant_digits(raw);
    let double_digits = significant_digits(&printed);
    literal_digits == double_digits
}

/// Significant decimal digits of a numeric literal, ignoring sign, exponent,
/// decimal point, and leading or trailing zeros.
fn significant_digits(raw: &str) -> String {
    let mantissa = raw
        .split(['e', 'E'])
        .next()
        .unwrap_or(raw)
        .trim_start_matches('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let trimmed = digits.trim_start_matches('0').trim_end_matches('0');
    trimmed.to_owned()
}

/// ECMAScript `Number::toString` (ES2015 --7.1.12.1), which is what RFC 8785
/// --3.2.2.3 defers to. Rust's own `{}` never uses exponent form, and JavaScript
/// switches at `1e21` and `1e-7`, so the digits have to be re-laid-out here.
fn ecmascript_number_to_string(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    if value < 0.0 {
        return format!("-{}", ecmascript_number_to_string(-value));
    }
    // `{:e}` gives the shortest round-trip digits as `d.ddde<exp>`.
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("Rust always emits an exponent in {:e}");
    let exponent: i32 = exponent.parse().expect("Rust emits a decimal exponent");
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let k = digits.len() as i32;
    // ECMAScript's `n` is the position of the decimal point: value = 0.digits x 10^n.
    let n = exponent + 1;

    if k <= n && n <= 21 {
        let mut out = digits.to_owned();
        out.push_str(&"0".repeat((n - k) as usize));
        out
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{}", "0".repeat((-n) as usize), digits)
    } else {
        let sign = if n > 0 { "+" } else { "-" };
        let magnitude = (n - 1).abs();
        if k == 1 {
            format!("{digits}e{sign}{magnitude}")
        } else {
            format!("{}.{}e{sign}{magnitude}", &digits[..1], &digits[1..])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_keys_are_rejected() {
        // The reason this parser exists: serde_json keeps the last one silently.
        let error = Parser::parse_one(br#"{"a":1,"a":2}"#).unwrap_err();
        assert_eq!(error.code, "BENCHMARK_JSON_INVALID");
        Parser::parse_one(br#"{"a":1,"b":2}"#).unwrap();
        // Nested duplicates are caught too.
        assert!(Parser::parse_one(br#"{"a":{"b":1,"b":2}}"#).is_err());
    }

    #[test]
    fn malformed_documents_fail_closed() {
        for rejected in [
            &br#"{"a":1"#[..],
            &br#"{"a" 1}"#[..],
            &br#"[1,]"#[..],
            &br#"{"a":01}"#[..],
            &br#"{"a":1.}"#[..],
            &br#"{"a":1e}"#[..],
            &br#"{"a":+1}"#[..],
            &br#"{"a":"unterminated}"#[..],
            &br#"{}{}"#[..],
            b"{\"a\":\"raw\ncontrol\"}",
        ] {
            assert!(
                Parser::parse_one(rejected).is_err(),
                "expected {:?} to be rejected",
                String::from_utf8_lossy(rejected)
            );
        }
    }

    #[test]
    fn numbers_keep_their_literal_text() {
        let value = Parser::parse_one(br#"{"a":1.0,"b":1e2,"c":-0.5}"#).unwrap();
        assert_eq!(value.get("a"), Some(&Json::Number("1.0".to_owned())));
        assert_eq!(value.get("b"), Some(&Json::Number("1e2".to_owned())));
        assert_eq!(value.get("c"), Some(&Json::Number("-0.5".to_owned())));
    }

    #[test]
    fn canonical_output_sorts_keys_and_matches_ecmascript_numbers() {
        // Inputs are built from escapes so this file stays ASCII; a raw byte
        // string cannot hold these characters anyway.
        let input = format!("{{\"b\":1,\"a\":2,\"{}\":3}}", '\u{e4}');
        let value = Parser::parse_one(input.as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8(canonicalize(&value).unwrap()).unwrap(),
            "{\"a\":2,\"b\":1,\"\u{e4}\":3}"
        );

        // RFC 8785 orders members by UTF-16 code units, which is not code-point
        // order: a supplementary character encodes as a surrogate pair starting
        // at 0xD800, so it sorts *before* a BMP character above 0xE000. Sorting by
        // code point would put them the other way round, so this is the case that
        // tells the two rules apart.
        let ordering_input = format!("{{\"{}\":1,\"{}\":2}}", '\u{fffd}', '\u{1f600}');
        let ordering = Parser::parse_one(ordering_input.as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8(canonicalize(&ordering).unwrap()).unwrap(),
            "{\"\u{1f600}\":2,\"\u{fffd}\":1}"
        );

        for (literal, expected) in [
            ("1.0", "1"),
            ("1e2", "100"),
            ("-0.5", "-0.5"),
            ("0", "0"),
            ("1e21", "1e+21"),
            ("1e-7", "1e-7"),
            ("0.000001", "0.000001"),
            ("8.514020219858878", "8.514020219858878"),
        ] {
            let parsed = Parser::parse_one(format!("[{literal}]").as_bytes()).unwrap();
            let bytes = canonicalize(&parsed).unwrap();
            assert_eq!(
                String::from_utf8(bytes).unwrap(),
                format!("[{expected}]"),
                "literal {literal}"
            );
        }
    }

    /// Python's non-standard float literals parse, and then refuse to
    /// canonicalize. Both halves matter: refusing to parse would block the real
    /// assets, and inventing a canonical form would silently rewrite the value.
    #[test]
    fn python_non_finite_literals_parse_but_never_canonicalize() {
        for literal in ["NaN", "Infinity", "-Infinity"] {
            let parsed = Parser::parse_one(format!("[{literal}]").as_bytes())
                .unwrap_or_else(|error| panic!("{literal} should parse: {}", error.message));
            assert_eq!(
                canonicalize(&parsed).unwrap_err().code,
                "BENCHMARK_NUMBER_NOT_REPRESENTABLE",
                "{literal} must have no canonical form"
            );
        }
        // Near-misses are still rejected outright rather than guessed at.
        for rejected in ["[Inf]", "[nan]", "[-Inf]", "[NaNN]", "[Infinityy]"] {
            assert!(
                Parser::parse_one(rejected.as_bytes()).is_err()
                    || canonicalize(&Parser::parse_one(rejected.as_bytes()).unwrap()).is_err(),
                "{rejected} must not parse into a usable value"
            );
        }
    }

    /// The case that motivated the round-trip check: these appear in the real
    /// MBPP+ payloads and have no RFC 8785 canonical form.
    #[test]
    fn numbers_beyond_double_precision_fail_closed() {
        for literal in [
            "6775685320645824322581483068371419745979053216268760300",
            "99999999999999999999999",
            "12345678901234567890",
        ] {
            let parsed = Parser::parse_one(format!("[{literal}]").as_bytes()).unwrap();
            let error = canonicalize(&parsed).unwrap_err();
            assert_eq!(
                error.code, "BENCHMARK_NUMBER_NOT_REPRESENTABLE",
                "literal {literal} should have no canonical form"
            );
        }
        // A large integer that *is* exactly a double canonicalizes normally.
        let exact = Parser::parse_one(b"[9007199254740992]").unwrap();
        assert_eq!(
            canonicalize(&exact).unwrap(),
            b"[9007199254740992]".to_vec()
        );
    }

    #[test]
    fn strings_are_escaped_minimally_and_surrogates_pair() {
        // Each escape isolated, so a failure names the case rather than the block.
        for (input, expected) in [
            ("[\"abc\"]", "[\"abc\"]"),
            ("[\"a\\\"b\"]", "[\"a\\\"b\"]"),
            ("[\"a\\\\b\"]", "[\"a\\\\b\"]"),
            ("[\"a\\nb\"]", "[\"a\\nb\"]"),
            ("[\"a\\tb\"]", "[\"a\\tb\"]"),
            // A legal escape that RFC 8785 re-emits unescaped.
            ("[\"a\\/b\"]", "[\"a/b\"]"),
        ] {
            let value = Parser::parse_one(input.as_bytes())
                .unwrap_or_else(|error| panic!("{input} did not parse: {}", error.message));
            assert_eq!(
                String::from_utf8(canonicalize(&value).unwrap()).unwrap(),
                expected,
                "canonicalizing {input}"
            );
        }

        // A surrogate pair decodes to one scalar and re-emits as raw UTF-8.
        // Written as escapes so this file stays ASCII and the pairing path is
        // exercised directly rather than incidentally.
        let paired = Parser::parse_one(br#"["\ud83d\ude00"]"#).unwrap();
        assert_eq!(
            String::from_utf8(canonicalize(&paired).unwrap()).unwrap(),
            "[\"\u{1f600}\"]"
        );
        // An unpaired high surrogate is not a Unicode scalar and must fail.
        assert!(Parser::parse_one(br#"["\ud83d"]"#).is_err());
    }
}
