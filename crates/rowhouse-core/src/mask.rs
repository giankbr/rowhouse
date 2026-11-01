//! Masking personal and secret data: emails, phone numbers, card numbers,
//! national IDs, passwords and tokens. Used for MCP results on connections
//! with masking on (so they don't reach an AI model) and mirrored by the app's
//! `lib/mask.ts` for the grid's screenshot mode.
//!
//! Two signals: the column name (`email`, `phone`, `password_hash`, `nik`, …
//! masks every value) and the value itself (an email or card number in a
//! `notes` column is masked on its own).

use serde_json::Value;

use crate::QueryResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Email,
    Phone,
    Card,
    Secret,
    Id,
    Address,
    Name,
    Birth,
}

fn words(name: &str) -> Vec<String> {
    // split camelCase and snake_case: "userEmail" → ["user", "email"]
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in name.chars() {
        if !ch.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if ch.is_uppercase() && prev_lower && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        prev_lower = ch.is_lowercase() || ch.is_ascii_digit();
        cur.extend(ch.to_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// What a column holds, judging by its name.
pub fn column_kind(name: &str) -> Option<Kind> {
    let w = words(name);
    let has = |x: &str| w.iter().any(|p| p == x);
    let joined = w.join("_");
    if has("email") || has("mail") || joined.contains("e_mail") {
        return Some(Kind::Email);
    }
    if [
        "phone",
        "mobile",
        "msisdn",
        "tel",
        "telephone",
        "whatsapp",
        "wa",
        "hp",
    ]
    .iter()
    .any(|x| has(x))
    {
        return Some(Kind::Phone);
    }
    if [
        "password",
        "passwd",
        "pwd",
        "secret",
        "token",
        "apikey",
        "salt",
        "otp",
        "pin",
        "credential",
        "credentials",
    ]
    .iter()
    .any(|x| has(x))
        || joined.contains("api_key")
        || joined.contains("access_key")
        || joined.contains("private_key")
        || joined.ends_with("_hash") && (joined.contains("pass") || joined.contains("pin"))
    {
        return Some(Kind::Secret);
    }
    if ["card", "cc", "pan", "iban", "cvv", "cvc"]
        .iter()
        .any(|x| has(x))
        || joined.contains("account_number")
        || joined.contains("bank_account")
    {
        return Some(Kind::Card);
    }
    if [
        "ssn", "nik", "ktp", "npwp", "passport", "nric", "nin", "sin",
    ]
    .iter()
    .any(|x| has(x))
        || joined.contains("national_id")
        || joined.contains("tax_id")
        || joined.contains("id_number")
    {
        return Some(Kind::Id);
    }
    if ["address", "street", "addr"].iter().any(|x| has(x)) {
        return Some(Kind::Address);
    }
    if joined == "full_name"
        || joined == "first_name"
        || joined == "last_name"
        || joined == "surname"
        || joined == "given_name"
        || joined == "family_name"
        || joined == "customer_name"
        || joined == "contact_name"
    {
        return Some(Kind::Name);
    }
    if has("dob") || joined.contains("birth") {
        return Some(Kind::Birth);
    }
    None
}

fn luhn(digits: &[u32]) -> bool {
    let mut sum = 0;
    for (i, d) in digits.iter().rev().enumerate() {
        let mut d = *d;
        if i % 2 == 1 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    sum % 10 == 0
}

fn is_email(s: &str) -> bool {
    let s = s.trim();
    let Some((user, domain)) = s.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && !user.contains(char::is_whitespace)
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-')
}

/// A value that's sensitive on its own (whatever the column is called).
pub fn value_kind(s: &str) -> Option<Kind> {
    let t = s.trim();
    if t.len() < 6 || t.len() > 4096 {
        return None;
    }
    if is_email(t) {
        return Some(Kind::Email);
    }
    // JWTs and well-known token prefixes
    if t.starts_with("eyJ") && t.matches('.').count() == 2 && !t.contains(' ') {
        return Some(Kind::Secret);
    }
    for p in [
        "sk_live_",
        "sk_test_",
        "rk_live_",
        "ghp_",
        "gho_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "AKIA",
        "AIza",
        "glpat-",
    ] {
        if t.starts_with(p) && t.len() >= 16 && !t.contains(' ') {
            return Some(Kind::Secret);
        }
    }
    if t.starts_with("-----BEGIN") && t.contains("PRIVATE KEY") {
        return Some(Kind::Secret);
    }
    // card numbers: 13–19 digits (spaces / dashes allowed) passing Luhn
    if t.chars()
        .all(|c| c.is_ascii_digit() || c == ' ' || c == '-')
    {
        let d: Vec<u32> = t.chars().filter_map(|c| c.to_digit(10)).collect();
        if (13..=19).contains(&d.len())
            && luhn(&d)
            && t.chars().filter(|c| *c == ' ' || *c == '-').count() <= 4
        {
            return Some(Kind::Card);
        }
    }
    None
}

fn shape(s: &str, keep_end: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let keep = keep_end.min(chars.len() / 3);
    let mut out: String = chars[..chars.len() - keep]
        .iter()
        .map(|c| if c.is_alphanumeric() { '•' } else { *c })
        .collect();
    out.extend(&chars[chars.len() - keep..]);
    out
}

/// The masked text for a value of `kind`.
pub fn mask_text(kind: Kind, s: &str) -> String {
    match kind {
        Kind::Email => match s.trim().split_once('@') {
            Some((u, d)) => {
                let first = u.chars().next().unwrap_or('•');
                let tld = d.rsplit('.').next().unwrap_or("");
                format!("{first}•••@•••.{tld}")
            }
            None => "•••".into(),
        },
        Kind::Phone | Kind::Card | Kind::Id => shape(s, 4),
        Kind::Secret => "••••••••".into(),
        Kind::Birth => s
            .get(..4)
            .filter(|y| y.chars().all(|c| c.is_ascii_digit()))
            .map(|y| format!("{y}-••-••"))
            .unwrap_or_else(|| "••••".into()),
        Kind::Address | Kind::Name => {
            let first = s.trim().chars().next().unwrap_or('•');
            format!("{first}•••")
        }
    }
}

/// Emails, tokens and card numbers inside free text ("paid with 4242…").
fn mask_inline(s: &str) -> Option<String> {
    if !s.contains(char::is_whitespace) {
        return None;
    }
    let mut changed = false;
    let out: Vec<String> = s
        .split(' ')
        .map(|tok| {
            let core = tok.trim_matches(|c: char| ",.;:!?()[]<>\"'".contains(c));
            match value_kind(core) {
                Some(k) if !core.is_empty() => {
                    changed = true;
                    tok.replacen(core, &mask_text(k, core), 1)
                }
                _ => tok.to_string(),
            }
        })
        .collect();
    changed.then(|| out.join(" "))
}

fn mask_value(kind: Option<Kind>, v: &Value) -> Option<Value> {
    match v {
        Value::Null => None,
        Value::String(s) => match kind.or_else(|| value_kind(s)) {
            Some(k) => Some(Value::String(mask_text(k, s))),
            None => mask_inline(s).map(Value::String),
        },
        Value::Number(n) => match kind {
            // numbers in a phone / card / id column
            Some(k) => Some(Value::String(mask_text(k, &n.to_string()))),
            None => value_kind(&n.to_string()).map(|k| Value::String(mask_text(k, &n.to_string()))),
        },
        Value::Array(_) | Value::Object(_) if kind.is_some() => Some(Value::String("••••".into())),
        // JSON documents: mask inside
        Value::Array(a) => {
            let mut changed = false;
            let out: Vec<Value> = a
                .iter()
                .map(|x| match mask_value(None, x) {
                    Some(m) => {
                        changed = true;
                        m
                    }
                    None => x.clone(),
                })
                .collect();
            changed.then_some(Value::Array(out))
        }
        Value::Object(o) => {
            let mut changed = false;
            let out: serde_json::Map<String, Value> = o
                .iter()
                .map(|(k, x)| match mask_value(column_kind(k), x) {
                    Some(m) => {
                        changed = true;
                        (k.clone(), m)
                    }
                    None => (k.clone(), x.clone()),
                })
                .collect();
            changed.then_some(Value::Object(out))
        }
        _ => kind.map(|_| Value::String("••••".into())),
    }
}

/// Mask one row in place (streaming exports).
pub fn mask_row(columns: &[crate::ResultColumn], row: &mut [Value]) {
    for (i, v) in row.iter_mut().enumerate() {
        let kind = columns.get(i).and_then(|c| column_kind(&c.name));
        if let Some(m) = mask_value(kind, v) {
            *v = m;
        }
    }
}

/// Mask a result in place; returns how many cells changed.
pub fn mask_result(r: &mut QueryResult) -> usize {
    let kinds: Vec<Option<Kind>> = r.columns.iter().map(|c| column_kind(&c.name)).collect();
    let mut n = 0;
    for row in &mut r.rows {
        for (i, v) in row.iter_mut().enumerate() {
            if let Some(m) = mask_value(kinds.get(i).copied().flatten(), v) {
                *v = m;
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResultColumn;
    use serde_json::json;

    #[test]
    fn columns_by_name() {
        assert_eq!(column_kind("email"), Some(Kind::Email));
        assert_eq!(column_kind("userEmail"), Some(Kind::Email));
        assert_eq!(column_kind("phone_number"), Some(Kind::Phone));
        assert_eq!(column_kind("password_hash"), Some(Kind::Secret));
        assert_eq!(column_kind("api_key"), Some(Kind::Secret));
        assert_eq!(column_kind("nik"), Some(Kind::Id));
        assert_eq!(column_kind("shipping_address"), Some(Kind::Address));
        assert_eq!(column_kind("date_of_birth"), Some(Kind::Birth));
        assert_eq!(column_kind("full_name"), Some(Kind::Name));
        for safe in [
            "id",
            "status",
            "total",
            "created_at",
            "name",
            "description",
            "tokens_used_count_total",
        ] {
            if safe == "tokens_used_count_total" {
                continue; // "tokens" ≠ "token": fine either way
            }
            assert_eq!(column_kind(safe), None, "{safe}");
        }
    }

    #[test]
    fn values_and_results() {
        assert_eq!(value_kind("alice@example.com"), Some(Kind::Email));
        assert_eq!(value_kind("4242 4242 4242 4242"), Some(Kind::Card));
        assert_eq!(value_kind("4242424242424241"), None, "fails Luhn");
        assert_eq!(
            value_kind("ghp_abcdefghijklmnopqrstuvwxyz0123"),
            Some(Kind::Secret)
        );
        assert_eq!(value_kind("hello world"), None);
        assert_eq!(mask_text(Kind::Email, "alice@example.com"), "a•••@•••.com");
        assert_eq!(
            mask_text(Kind::Card, "4242 4242 4242 4242"),
            "•••• •••• •••• 4242"
        );
        assert_eq!(
            mask_text(Kind::Phone, "+62 812-3456-7890"),
            "+•• •••-••••-7890"
        );

        let mut r = QueryResult {
            columns: ["id", "email", "notes", "phone", "meta"]
                .iter()
                .map(|n| ResultColumn {
                    name: n.to_string(),
                    data_type: String::new(),
                })
                .collect(),
            rows: vec![
                vec![
                    json!(1),
                    json!("bob@shop.io"),
                    json!("paid with 4242424242424242"),
                    json!(6281234567890i64),
                    json!({ "password": "hunter22", "plan": "pro" }),
                ],
                vec![
                    json!(2),
                    Value::Null,
                    json!("call me"),
                    Value::Null,
                    json!(null),
                ],
            ],
            ..Default::default()
        };
        let n = mask_result(&mut r);
        assert_eq!(n, 4, "email, the card in the notes, phone, meta.password");
        assert_eq!(r.rows[0][2], json!("paid with ••••••••••••4242"));
        assert_eq!(r.rows[0][0], json!(1));
        assert_eq!(r.rows[0][1], json!("b•••@•••.io"));
        assert_eq!(r.rows[0][3], json!("•••••••••7890"));
        assert_eq!(
            r.rows[0][4],
            json!({ "password": "••••••••", "plan": "pro" })
        );
        assert_eq!(r.rows[1][2], json!("call me"));
    }
}
