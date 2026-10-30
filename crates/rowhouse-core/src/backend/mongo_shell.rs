//! A small mongosh-style parser: `db.users.find({ age: { $gt: 30 } }).sort({ _id: -1 }).limit(10)`.
//!
//! Arguments are JavaScript-ish literals — unquoted keys, single quotes,
//! trailing commas, comments, `/regex/i` and the usual shell helpers
//! (`ObjectId("…")`, `ISODate("…")`, `new Date()`, `NumberLong(…)`,
//! `NumberDecimal("…")`, `UUID("…")`). No expressions or variables.

use mongodb::bson::{
    oid::ObjectId, spec::BinarySubtype, Binary, Bson, DateTime, Decimal128, Document, Regex,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    ShowCollections,
    ShowDatabases,
    /// `db.runCommand({...})` / `db.adminCommand({...})`
    Command {
        admin: bool,
        doc: Document,
    },
    /// `db.getCollectionNames()`
    CollectionNames,
    /// `db.<coll>.<method>(args)[.chain(args)]*`
    Call {
        coll: String,
        method: String,
        args: Vec<Bson>,
        chain: Vec<(String, Vec<Bson>)>,
    },
}

pub fn parse(src: &str) -> Result<Vec<Stmt>, String> {
    let mut p = P {
        s: src.chars().collect(),
        i: 0,
    };
    let mut out = Vec::new();
    loop {
        p.ws();
        while p.eat(';') {
            p.ws();
        }
        if p.done() {
            break;
        }
        out.push(p.stmt()?);
    }
    Ok(out)
}

/// Parse one literal value (used for filters typed in the grid, e.g. `{a: 1}`).
pub fn parse_value(src: &str) -> Result<Bson, String> {
    let mut p = P {
        s: src.chars().collect(),
        i: 0,
    };
    let v = p.value()?;
    p.ws();
    if !p.done() {
        return Err(p.err("unexpected text after value"));
    }
    Ok(v)
}

struct P {
    s: Vec<char>,
    i: usize,
}

fn ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}
fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

impl P {
    fn done(&self) -> bool {
        self.i >= self.s.len()
    }
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }
    fn peek_at(&self, n: usize) -> Option<char> {
        self.s.get(self.i + n).copied()
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn line_col(&self) -> (usize, usize) {
        let mut line = 1;
        let mut col = 1;
        for &c in &self.s[..self.i.min(self.s.len())] {
            if c == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }
    fn err(&self, msg: &str) -> String {
        let (l, c) = self.line_col();
        format!("{msg} (line {l}, column {c})")
    }
    fn expect(&mut self, c: char) -> Result<(), String> {
        self.ws();
        if self.eat(c) {
            Ok(())
        } else {
            Err(self.err(&format!("expected '{c}'")))
        }
    }
    /// whitespace and comments
    fn ws(&mut self) {
        loop {
            while self.peek().is_some_and(char::is_whitespace) {
                self.i += 1;
            }
            if self.peek() == Some('/') && self.peek_at(1) == Some('/') {
                while self.peek().is_some_and(|c| c != '\n') {
                    self.i += 1;
                }
            } else if self.peek() == Some('/') && self.peek_at(1) == Some('*') {
                self.i += 2;
                while !self.done() && !(self.peek() == Some('*') && self.peek_at(1) == Some('/')) {
                    self.i += 1;
                }
                self.i = (self.i + 2).min(self.s.len());
            } else {
                break;
            }
        }
    }
    fn ident(&mut self) -> Option<String> {
        self.ws();
        if !self.peek().is_some_and(ident_start) {
            return None;
        }
        let st = self.i;
        while self.peek().is_some_and(ident_char) {
            self.i += 1;
        }
        Some(self.s[st..self.i].iter().collect())
    }
    fn need_ident(&mut self, what: &str) -> Result<String, String> {
        self.ident()
            .ok_or_else(|| self.err(&format!("expected {what}")))
    }

    fn stmt(&mut self) -> Result<Stmt, String> {
        let first = self.need_ident("`db.<collection>.<method>(…)` or `show collections`")?;
        if first == "show" {
            let what = self.need_ident("collections or dbs")?;
            return match what.as_str() {
                "collections" | "tables" => Ok(Stmt::ShowCollections),
                "dbs" | "databases" => Ok(Stmt::ShowDatabases),
                _ => Err(self.err(&format!("unknown `show {what}`"))),
            };
        }
        if first != "db" {
            return Err(self.err(&format!("statements start with `db.` (got `{first}`)")));
        }
        // path: db.a.b.c.method(  /  db["name"].method(  /  db.getCollection("x").method(
        let mut path: Vec<String> = Vec::new();
        let coll: String;
        let method: String;
        loop {
            self.ws();
            if self.eat('[') {
                self.ws();
                let name = self.string()?;
                self.expect(']')?;
                path.push(name);
                continue;
            }
            if !self.eat('.') {
                return Err(self.err("expected `.`"));
            }
            let name = self.need_ident("a name")?;
            self.ws();
            if self.peek() == Some('(') {
                if path.is_empty() {
                    // db-level call
                    let args = self.args()?;
                    return match name.as_str() {
                        "getCollection" => {
                            let Some(Bson::String(c)) = args.into_iter().next() else {
                                return Err(self.err("getCollection needs a name"));
                            };
                            path.push(c);
                            continue;
                        }
                        "getCollectionNames" => Ok(Stmt::CollectionNames),
                        "runCommand" | "adminCommand" => match args.into_iter().next() {
                            Some(Bson::Document(doc)) => Ok(Stmt::Command {
                                admin: name == "adminCommand",
                                doc,
                            }),
                            Some(Bson::String(s)) => {
                                let mut doc = Document::new();
                                doc.insert(s, 1);
                                Ok(Stmt::Command {
                                    admin: name == "adminCommand",
                                    doc,
                                })
                            }
                            _ => Err(self.err("runCommand needs a document")),
                        },
                        other => Err(self.err(&format!("unsupported `db.{other}()`"))),
                    };
                }
                coll = path.join(".");
                method = name;
                break;
            }
            path.push(name);
        }
        let args = self.args()?;
        let mut chain = Vec::new();
        loop {
            let save = self.i;
            self.ws();
            if self.eat('.') {
                let name = self.need_ident("a cursor method")?;
                let a = self.args()?;
                chain.push((name, a));
            } else {
                self.i = save;
                break;
            }
        }
        Ok(Stmt::Call {
            coll,
            method,
            args,
            chain,
        })
    }

    fn args(&mut self) -> Result<Vec<Bson>, String> {
        self.expect('(')?;
        let mut out = Vec::new();
        loop {
            self.ws();
            if self.eat(')') {
                return Ok(out);
            }
            out.push(self.value()?);
            self.ws();
            if self.eat(',') {
                continue;
            }
            self.expect(')')?;
            return Ok(out);
        }
    }

    fn string(&mut self) -> Result<String, String> {
        let q = match self.peek() {
            Some(c @ ('"' | '\'' | '`')) => c,
            _ => return Err(self.err("expected a string")),
        };
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.err("unterminated string"));
            };
            self.i += 1;
            if c == q {
                return Ok(out);
            }
            if c != '\\' {
                out.push(c);
                continue;
            }
            let Some(e) = self.peek() else {
                return Err(self.err("unterminated string"));
            };
            self.i += 1;
            match e {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                '0' => out.push('\0'),
                'u' => {
                    let hex: String = self.s[self.i..(self.i + 4).min(self.s.len())]
                        .iter()
                        .collect();
                    let cp =
                        u32::from_str_radix(&hex, 16).map_err(|_| self.err("bad \\u escape"))?;
                    self.i += 4;
                    out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                }
                other => out.push(other),
            }
        }
    }

    fn number(&mut self) -> Result<Bson, String> {
        let st = self.i;
        if self.peek() == Some('-') || self.peek() == Some('+') {
            self.i += 1;
        }
        let mut float = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.i += 1;
            } else if c == '.' || c == 'e' || c == 'E' {
                float = true;
                self.i += 1;
                if (c == 'e' || c == 'E') && matches!(self.peek(), Some('-' | '+')) {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
        let txt: String = self.s[st..self.i].iter().collect();
        if txt == "-" || txt == "+" {
            if self.ident().as_deref() == Some("Infinity") {
                return Ok(Bson::Double(if txt == "-" {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                }));
            }
            return Err(self.err("expected a number"));
        }
        if !float {
            if let Ok(n) = txt.parse::<i64>() {
                return Ok(match i32::try_from(n) {
                    Ok(n) => Bson::Int32(n),
                    Err(_) => Bson::Int64(n),
                });
            }
        }
        txt.parse::<f64>()
            .map(Bson::Double)
            .map_err(|_| self.err(&format!("bad number `{txt}`")))
    }

    fn regex(&mut self) -> Result<Bson, String> {
        self.i += 1; // '/'
        let mut pattern = String::new();
        let mut in_class = false;
        loop {
            let Some(c) = self.peek() else {
                return Err(self.err("unterminated regex"));
            };
            self.i += 1;
            match c {
                '\\' => {
                    pattern.push(c);
                    if let Some(n) = self.peek() {
                        pattern.push(n);
                        self.i += 1;
                    }
                }
                '[' => {
                    in_class = true;
                    pattern.push(c)
                }
                ']' => {
                    in_class = false;
                    pattern.push(c)
                }
                '/' if !in_class => break,
                _ => pattern.push(c),
            }
        }
        let mut options = String::new();
        while let Some(c) = self.peek().filter(|c| c.is_ascii_alphabetic()) {
            options.push(c);
            self.i += 1;
        }
        let mut o: Vec<char> = options.chars().collect();
        o.sort();
        Ok(Bson::RegularExpression(Regex {
            pattern,
            options: o.into_iter().collect(),
        }))
    }

    fn value(&mut self) -> Result<Bson, String> {
        self.ws();
        match self.peek() {
            None => Err(self.err("expected a value")),
            Some('{') => self.object().map(Bson::Document),
            Some('[') => {
                self.i += 1;
                let mut arr = Vec::new();
                loop {
                    self.ws();
                    if self.eat(']') {
                        return Ok(Bson::Array(arr));
                    }
                    arr.push(self.value()?);
                    self.ws();
                    if !self.eat(',') {
                        self.expect(']')?;
                        return Ok(Bson::Array(arr));
                    }
                }
            }
            Some('"' | '\'' | '`') => self.string().map(Bson::String),
            Some('/') => self.regex(),
            Some(c) if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' => self.number(),
            Some(c) if ident_start(c) => {
                let mut name = self.ident().unwrap_or_default();
                if name == "new" {
                    name = self.need_ident("a constructor")?;
                }
                match name.as_str() {
                    "true" => return Ok(Bson::Boolean(true)),
                    "false" => return Ok(Bson::Boolean(false)),
                    "null" | "undefined" => return Ok(Bson::Null),
                    "Infinity" => return Ok(Bson::Double(f64::INFINITY)),
                    "NaN" => return Ok(Bson::Double(f64::NAN)),
                    "MinKey" | "MaxKey" => {
                        self.ws();
                        if self.peek() == Some('(') {
                            self.args()?;
                        }
                        return Ok(if name == "MinKey" {
                            Bson::MinKey
                        } else {
                            Bson::MaxKey
                        });
                    }
                    _ => {}
                }
                let args = self.args()?;
                self.helper(&name, args)
            }
            Some(c) => Err(self.err(&format!("unexpected `{c}`"))),
        }
    }

    fn helper(&self, name: &str, args: Vec<Bson>) -> Result<Bson, String> {
        let first = args.into_iter().next();
        let s = |b: &Option<Bson>| match b {
            Some(Bson::String(s)) => Some(s.clone()),
            Some(Bson::Int32(n)) => Some(n.to_string()),
            Some(Bson::Int64(n)) => Some(n.to_string()),
            Some(Bson::Double(n)) => Some(n.to_string()),
            _ => None,
        };
        match name {
            "ObjectId" | "ObjectID" => match s(&first) {
                Some(hex) => ObjectId::parse_str(&hex)
                    .map(Bson::ObjectId)
                    .map_err(|_| self.err(&format!("invalid ObjectId `{hex}`"))),
                None => Ok(Bson::ObjectId(ObjectId::new())),
            },
            "ISODate" | "Date" => match first {
                None => Ok(Bson::DateTime(DateTime::now())),
                Some(Bson::Int32(ms)) => Ok(Bson::DateTime(DateTime::from_millis(ms as i64))),
                Some(Bson::Int64(ms)) => Ok(Bson::DateTime(DateTime::from_millis(ms))),
                Some(Bson::Double(ms)) => Ok(Bson::DateTime(DateTime::from_millis(ms as i64))),
                Some(Bson::String(t)) => parse_date(&t)
                    .map(Bson::DateTime)
                    .ok_or_else(|| self.err(&format!("invalid date `{t}`"))),
                _ => Err(self.err("invalid date")),
            },
            "NumberInt" | "Int32" => s(&first)
                .and_then(|t| t.parse::<i32>().ok())
                .map(Bson::Int32)
                .ok_or_else(|| self.err("invalid NumberInt")),
            "NumberLong" | "Long" => s(&first)
                .and_then(|t| t.parse::<i64>().ok())
                .map(Bson::Int64)
                .ok_or_else(|| self.err("invalid NumberLong")),
            "NumberDecimal" | "Decimal128" => s(&first)
                .and_then(|t| t.parse::<Decimal128>().ok())
                .map(Bson::Decimal128)
                .ok_or_else(|| self.err("invalid NumberDecimal")),
            "Double" => s(&first)
                .and_then(|t| t.parse::<f64>().ok())
                .map(Bson::Double)
                .ok_or_else(|| self.err("invalid Double")),
            "UUID" => {
                let hex: String = s(&first)
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| c.is_ascii_hexdigit())
                    .collect();
                if hex.len() != 32 {
                    return Err(self.err("invalid UUID"));
                }
                let bytes = (0..16)
                    .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0))
                    .collect();
                Ok(Bson::Binary(Binary {
                    subtype: BinarySubtype::Uuid,
                    bytes,
                }))
            }
            "RegExp" => match first {
                Some(Bson::String(p)) => Ok(Bson::RegularExpression(Regex {
                    pattern: p,
                    options: String::new(),
                })),
                _ => Err(self.err("invalid RegExp")),
            },
            other => Err(self.err(&format!("unknown helper `{other}()`"))),
        }
    }

    fn object(&mut self) -> Result<Document, String> {
        self.i += 1; // '{'
        let mut doc = Document::new();
        loop {
            self.ws();
            if self.eat('}') {
                return Ok(doc);
            }
            let key = match self.peek() {
                Some('"' | '\'' | '`') => self.string()?,
                Some(c) if c.is_ascii_digit() => {
                    let st = self.i;
                    while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                        self.i += 1;
                    }
                    self.s[st..self.i].iter().collect()
                }
                _ => {
                    // unquoted keys may contain dots: { "a.b": 1 } is common unquoted too
                    let mut k = self.need_ident("a key")?;
                    while self.peek() == Some('.') && self.peek_at(1).is_some_and(ident_char) {
                        self.i += 1;
                        k.push('.');
                        while self.peek().is_some_and(ident_char) {
                            k.push(self.s[self.i]);
                            self.i += 1;
                        }
                    }
                    k
                }
            };
            self.expect(':')?;
            let v = self.value()?;
            doc.insert(key, v);
            self.ws();
            if !self.eat(',') {
                self.expect('}')?;
                return Ok(doc);
            }
        }
    }
}

/// RFC 3339, or a bare date / date-time without zone (taken as UTC).
pub fn parse_date(t: &str) -> Option<DateTime> {
    let t = t.trim();
    if let Ok(d) = DateTime::parse_rfc3339_str(t) {
        return Some(d);
    }
    let fixed = if t.len() == 10 {
        format!("{t}T00:00:00Z")
    } else {
        format!("{}Z", t.replace(' ', "T"))
    };
    DateTime::parse_rfc3339_str(fixed).ok()
}

/// Shell-style rendering (for previews and the "query" shown above a page).
pub fn shell(v: &Bson) -> String {
    match v {
        Bson::Document(d) => {
            if d.is_empty() {
                return "{}".into();
            }
            let parts: Vec<String> = d
                .iter()
                .map(|(k, v)| format!("{}: {}", shell_key(k), shell(v)))
                .collect();
            format!("{{ {} }}", parts.join(", "))
        }
        Bson::Array(a) => format!("[{}]", a.iter().map(shell).collect::<Vec<_>>().join(", ")),
        Bson::String(s) => serde_json::to_string(s).unwrap_or_default(),
        Bson::ObjectId(o) => format!("ObjectId(\"{o}\")"),
        Bson::DateTime(d) => format!(
            "ISODate(\"{}\")",
            d.try_to_rfc3339_string().unwrap_or_default()
        ),
        Bson::Int64(n) => format!("NumberLong({n})"),
        Bson::Decimal128(d) => format!("NumberDecimal(\"{d}\")"),
        Bson::RegularExpression(r) => format!("/{}/{}", r.pattern, r.options),
        Bson::Null => "null".into(),
        other => other.to_string(),
    }
}

fn shell_key(k: &str) -> String {
    if !k.is_empty() && k.chars().next().is_some_and(ident_start) && k.chars().all(ident_char) {
        k.to_string()
    } else {
        serde_json::to_string(k).unwrap_or_default()
    }
}
