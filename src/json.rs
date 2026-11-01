//! Small, bounded JSON reader for GitHub's responses.

#[derive(Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(fields) => fields
                .iter()
                .rev()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn count(&self) -> Option<u64> {
        match self {
            Self::Number(value) => value.parse().ok(),
            _ => None,
        }
    }

    pub fn array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }
}

pub fn parse(input: &str) -> Result<Value, String> {
    let mut reader = Reader {
        input: input.as_bytes(),
        pos: 0,
    };
    let value = reader.value(0)?;
    reader.space();
    if reader.pos != reader.input.len() {
        return Err(reader.error("trailing data"));
    }
    Ok(value)
}

struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn error(&self, message: &str) -> String {
        format!("invalid JSON at byte {}: {message}", self.pos)
    }

    fn space(&mut self) {
        while matches!(self.input.get(self.pos), Some(b' ' | b'\r' | b'\n' | b'\t')) {
            self.pos += 1;
        }
    }

    fn take(&mut self, byte: u8) -> bool {
        if self.input.get(self.pos) == Some(&byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.take(byte) {
            Ok(())
        } else {
            Err(self.error("unexpected character"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > 128 {
            return Err(self.error("nesting limit exceeded"));
        }
        self.space();
        match self.input.get(self.pos) {
            Some(b'"') => self.string().map(Value::String),
            Some(b'{') => {
                self.pos += 1;
                self.space();
                let mut fields = Vec::new();
                if self.take(b'}') {
                    return Ok(Value::Object(fields));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    self.space();
                    self.expect(b':')?;
                    fields.push((key, self.value(depth + 1)?));
                    self.space();
                    if self.take(b'}') {
                        break;
                    }
                    self.expect(b',')?;
                }
                Ok(Value::Object(fields))
            }
            Some(b'[') => {
                self.pos += 1;
                self.space();
                let mut items = Vec::new();
                if self.take(b']') {
                    return Ok(Value::Array(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.space();
                    if self.take(b']') {
                        break;
                    }
                    self.expect(b',')?;
                }
                Ok(Value::Array(items))
            }
            Some(b'n') => self.literal(b"null", Value::Null),
            Some(b't') => self.literal(b"true", Value::Bool(true)),
            Some(b'f') => self.literal(b"false", Value::Bool(false)),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.error("expected a value")),
        }
    }

    fn literal(&mut self, literal: &[u8], value: Value) -> Result<Value, String> {
        if self.input.get(self.pos..self.pos + literal.len()) == Some(literal) {
            self.pos += literal.len();
            Ok(value)
        } else {
            Err(self.error("invalid literal"))
        }
    }

    fn digits(&mut self) -> Result<(), String> {
        let start = self.pos;
        while self.input.get(self.pos).is_some_and(u8::is_ascii_digit) {
            self.pos += 1;
        }
        if self.pos == start {
            Err(self.error("expected digits"))
        } else {
            Ok(())
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        self.take(b'-');
        if !self.take(b'0') {
            self.digits()?;
        }
        if self.take(b'.') {
            self.digits()?;
        }
        if self.take(b'e') || self.take(b'E') {
            if !self.take(b'+') {
                self.take(b'-');
            }
            self.digits()?;
        }
        Ok(Value::Number(
            String::from_utf8(self.input[start..self.pos].to_vec()).unwrap(),
        ))
    }

    fn hex(&mut self) -> Result<u16, String> {
        let bytes = self
            .input
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.error("short Unicode escape"))?;
        if !bytes.iter().all(u8::is_ascii_hexdigit) {
            return Err(self.error("invalid Unicode escape"));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| self.error("invalid Unicode escape"))?;
        let value =
            u16::from_str_radix(text, 16).map_err(|_| self.error("invalid Unicode escape"))?;
        self.pos += 4;
        Ok(value)
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            let byte = *self
                .input
                .get(self.pos)
                .ok_or_else(|| self.error("unterminated string"))?;
            self.pos += 1;
            match byte {
                b'"' => return String::from_utf8(out).map_err(|_| self.error("invalid UTF-8")),
                b'\\' => {
                    let escape = *self
                        .input
                        .get(self.pos)
                        .ok_or_else(|| self.error("short escape"))?;
                    self.pos += 1;
                    match escape {
                        b'"' | b'\\' | b'/' => out.push(escape),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let high = self.hex()?;
                            let code = if (0xd800..=0xdbff).contains(&high) {
                                self.expect(b'\\')?;
                                self.expect(b'u')?;
                                let low = self.hex()?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return Err(self.error("invalid surrogate pair"));
                                }
                                0x10000 + ((high as u32 - 0xd800) << 10) + (low as u32 - 0xdc00)
                            } else {
                                high as u32
                            };
                            let ch = char::from_u32(code)
                                .ok_or_else(|| self.error("invalid Unicode scalar"))?;
                            out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        _ => return Err(self.error("invalid escape")),
                    }
                }
                0..=31 => return Err(self.error("unescaped control character")),
                _ => out.push(byte),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_lookup_preserves_last_duplicate_key_and_nested_fields() {
        let data = parse(r#"{"login":"old","nested":{"login":"inside"},"login":"new"}"#).unwrap();
        assert_eq!(data.get("login").and_then(Value::text), Some("new"));
        assert_eq!(
            data.get("nested")
                .and_then(|v| v.get("login"))
                .and_then(Value::text),
            Some("inside")
        );
        assert!(data.get("missing").is_none());
    }

    #[test]
    fn reads_github_strings_and_large_ids() {
        let data = parse(r#"{"id":9007199254740993,"bio":"Señor \uD83E\uDDD9\n\"Rust\"","location":null,"items":[true,false,-1.2e+3]}"#).unwrap();
        assert_eq!(
            data.get("id").and_then(Value::count),
            Some(9007199254740993)
        );
        assert_eq!(
            data.get("bio").and_then(Value::text),
            Some("Señor 🧙\n\"Rust\"")
        );
        assert_eq!(data.get("location"), Some(&Value::Null));
    }

    #[test]
    fn rejects_malformed_and_excessively_nested_input() {
        for input in [
            "",
            "01",
            "-",
            "1.",
            "1e",
            "[1,]",
            "{\"a\":1,}",
            "true false",
            r#""\uD800""#,
            r#""\uDC00""#,
            r#""\q""#,
            r#""\u+123""#,
            "\"\n\"",
        ] {
            assert!(parse(input).is_err(), "accepted {input:?}");
        }
        assert!(parse(&format!("{}0{}", "[".repeat(130), "]".repeat(130))).is_err());
    }
}
