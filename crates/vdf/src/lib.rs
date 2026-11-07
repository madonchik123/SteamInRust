//! Parser for Valve's text KeyValues format, used by `libraryfolders.vdf`,
//! `appmanifest_*.acf`, `loginusers.vdf`, `localconfig.vdf` and friends.
//!
//! Keys are matched case-insensitively, because Steam is inconsistent about it.

use std::fmt;

pub mod binary;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Obj(Obj),
}

/// An ordered list of key/value pairs. Duplicate keys are allowed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Obj(pub Vec<(String, Value)>);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unexpected end of input")]
    Eof,
    #[error("unexpected '{0}' at byte {1}")]
    Unexpected(char, usize),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            Value::Obj(_) => None,
        }
    }
    pub fn as_obj(&self) -> Option<&Obj> {
        match self {
            Value::Obj(o) => Some(o),
            Value::Str(_) => None,
        }
    }
    /// Shortcut: `value.get("a")` when value is an object.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_obj()?.get(key)
    }
}

impl Obj {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }
    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }
    pub fn obj(&self, key: &str) -> Option<&Obj> {
        self.get(key)?.as_obj()
    }
    pub fn u64(&self, key: &str) -> Option<u64> {
        self.str(key)?.trim().parse().ok()
    }
    /// Follow a path of nested object keys.
    pub fn path(&self, keys: &[&str]) -> Option<&Obj> {
        keys.iter().try_fold(self, |o, k| o.obj(k))
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Str(s) => write!(f, "{s:?}"),
            Value::Obj(o) => write!(f, "{{ {} entries }}", o.0.len()),
        }
    }
}

/// Write a document in Valve's text format (tabs, quoted keys/values).
pub fn write(obj: &Obj) -> String {
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }
    fn go(o: &Obj, depth: usize, out: &mut String) {
        let pad = "\t".repeat(depth);
        for (k, v) in &o.0 {
            match v {
                Value::Str(s) => out.push_str(&format!("{pad}\"{}\"\t\t\"{}\"\n", esc(k), esc(s))),
                Value::Obj(inner) => {
                    out.push_str(&format!("{pad}\"{}\"\n{pad}{{\n", esc(k)));
                    go(inner, depth + 1, out);
                    out.push_str(&format!("{pad}}}\n"));
                }
            }
        }
    }
    let mut out = String::new();
    go(obj, 0, &mut out);
    out
}

impl Obj {
    pub fn with(mut self, key: &str, value: impl Into<String>) -> Self {
        self.0.push((key.into(), Value::Str(value.into())));
        self
    }
    pub fn with_obj(mut self, key: &str, value: Obj) -> Self {
        self.0.push((key.into(), Value::Obj(value)));
        self
    }
}

/// Parse a whole document. The result is the top-level object, which usually
/// holds a single root key such as `"libraryfolders"` or `"AppState"`.
pub fn parse(input: &str) -> Result<Obj, Error> {
    let mut p = Parser { src: input.as_bytes(), pos: 0 };
    p.object(true)
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

enum Tok {
    Str(String),
    Open,
    Close,
}

impl Parser<'_> {
    fn object(&mut self, top: bool) -> Result<Obj, Error> {
        let mut out = Vec::new();
        loop {
            let key = match self.next()? {
                None if top => return Ok(Obj(out)),
                None => return Err(Error::Eof),
                Some(Tok::Close) if !top => return Ok(Obj(out)),
                Some(Tok::Close) => return Err(Error::Unexpected('}', self.pos)),
                Some(Tok::Open) => return Err(Error::Unexpected('{', self.pos)),
                Some(Tok::Str(s)) => s,
            };
            let value = match self.next()? {
                Some(Tok::Str(s)) => Value::Str(s),
                Some(Tok::Open) => Value::Obj(self.object(false)?),
                Some(Tok::Close) => return Err(Error::Unexpected('}', self.pos)),
                None => return Err(Error::Eof),
            };
            out.push((key, value));
        }
    }

    fn next(&mut self) -> Result<Option<Tok>, Error> {
        loop {
            self.skip_ws();
            let Some(&c) = self.src.get(self.pos) else {
                return Ok(None);
            };
            match c {
                b'/' if self.src.get(self.pos + 1) == Some(&b'/') => {
                    while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                // Platform conditionals like [$WIN32] are ignored.
                b'[' => {
                    while self.pos < self.src.len() && self.src[self.pos] != b']' {
                        self.pos += 1;
                    }
                    self.pos += 1;
                }
                b'{' => {
                    self.pos += 1;
                    return Ok(Some(Tok::Open));
                }
                b'}' => {
                    self.pos += 1;
                    return Ok(Some(Tok::Close));
                }
                b'"' => return self.quoted().map(|s| Some(Tok::Str(s))),
                _ => return Ok(Some(Tok::Str(self.bare()))),
            }
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn quoted(&mut self) -> Result<String, Error> {
        self.pos += 1; // opening quote
        let mut buf = Vec::new();
        while let Some(&c) = self.src.get(self.pos) {
            self.pos += 1;
            match c {
                b'"' => return Ok(String::from_utf8_lossy(&buf).into_owned()),
                b'\\' => {
                    let esc = *self.src.get(self.pos).ok_or(Error::Eof)?;
                    self.pos += 1;
                    buf.push(match esc {
                        b'n' => b'\n',
                        b't' => b'\t',
                        other => other,
                    });
                }
                _ => buf.push(c),
            }
        }
        Err(Error::Eof)
    }

    fn bare(&mut self) -> String {
        let start = self.pos;
        while let Some(&c) = self.src.get(self.pos) {
            if c.is_ascii_whitespace() || matches!(c, b'{' | b'}' | b'"') {
                break;
            }
            self.pos += 1;
        }
        String::from_utf8_lossy(&self.src[start..self.pos]).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_library_folders() {
        let doc = r#"
"libraryfolders"
{
    // comment
    "0"
    {
        "path"      "C:\\Program Files (x86)\\Steam"
        "apps"
        {
            "228980"    "123"
            "730"       "456"
        }
    }
}"#;
        let root = parse(doc).unwrap();
        let lf = root.obj("LibraryFolders").unwrap();
        let zero = lf.obj("0").unwrap();
        assert_eq!(zero.str("path"), Some(r"C:\Program Files (x86)\Steam"));
        assert_eq!(zero.obj("apps").unwrap().u64("730"), Some(456));
    }

    #[test]
    fn handles_bare_tokens_and_conditionals() {
        let root = parse("a { b c [$WIN32] d \"e f\" }").unwrap();
        let a = root.obj("a").unwrap();
        assert_eq!(a.str("b"), Some("c"));
        assert_eq!(a.str("d"), Some("e f"));
    }

    #[test]
    fn write_roundtrip() {
        let o = Obj::default().with_obj("AppState", Obj::default().with("appid", "440").with("installdir", "Team \"F\" 2"));
        let text = write(&o);
        let back = parse(&text).unwrap();
        assert_eq!(back.path(&["AppState"]).unwrap().str("installdir"), Some("Team \"F\" 2"));
    }

    #[test]
    fn errors_on_unclosed() {
        assert!(parse("a { b c").is_err());
    }
}
