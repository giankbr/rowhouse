use ring::signature::{Ed25519KeyPair, KeyPair};
use rowhouse_core::license::{verify_with, License};

fn b64url(b: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

#[test]
fn signed_keys_verify_and_tampering_fails() {
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let kp = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let lic = License {
        email: "a@b.c".into(),
        plan: "pro".into(),
        seats: 1,
        issued: 1_800_000_000,
        updates_until: 1_831_536_000,
        id: "L1".into(),
    };
    let payload = serde_json::to_vec(&lic).unwrap();
    let key = format!(
        "RH1-{}.{}",
        b64url(&payload),
        b64url(kp.sign(&payload).as_ref())
    );
    assert_eq!(verify_with(&key, kp.public_key().as_ref()).unwrap(), lic);
    // padded / whitespace-wrapped keys from email still work
    assert!(verify_with(&format!("  {key}\n"), kp.public_key().as_ref()).is_ok());

    // edited payload (team instead of pro) → signature mismatch
    let forged = serde_json::to_vec(&License {
        plan: "team".into(),
        ..lic.clone()
    })
    .unwrap();
    let bad = format!("RH1-{}.{}", b64url(&forged), key.split('.').nth(1).unwrap());
    assert!(verify_with(&bad, kp.public_key().as_ref())
        .unwrap_err()
        .to_string()
        .contains("signature"));
    // another key pair can't verify it
    let other =
        Ed25519KeyPair::from_pkcs8(Ed25519KeyPair::generate_pkcs8(&rng).unwrap().as_ref()).unwrap();
    assert!(verify_with(&key, other.public_key().as_ref()).is_err());
    assert!(verify_with("hello", kp.public_key().as_ref()).is_err());
}

#[test]
fn community_builds_are_unlocked() {
    // tests are built without ROWHOUSE_LICENSE_PUBKEY
    let s = rowhouse_core::license::status();
    assert!(s.community_build && s.pro);
    assert!(rowhouse_core::license::require_pro("x").is_ok());
}

/// Cross-check with scripts/license.mjs: ROWHOUSE_TEST_LICENSE_KEY + ROWHOUSE_TEST_LICENSE_PUB.
#[test]
fn keys_from_the_node_script_verify() {
    let (Ok(key), Ok(pubkey)) = (
        std::env::var("ROWHOUSE_TEST_LICENSE_KEY"),
        std::env::var("ROWHOUSE_TEST_LICENSE_PUB"),
    ) else {
        return;
    };
    let std: String = pubkey
        .chars()
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    let pk = base64_std(&std);
    let lic = verify_with(&key, &pk).unwrap();
    assert_eq!(lic.email, "buyer@example.com");
    assert_eq!(lic.plan, "pro");
}

fn base64_std(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    } as u32;
    let b: Vec<u8> = s.bytes().filter(|c| *c != b'=').collect();
    let mut out = vec![];
    for ch in b.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in ch.iter().enumerate() {
            acc |= val(c) << (18 - 6 * i);
        }
        out.push((acc >> 16) as u8);
        if ch.len() > 2 {
            out.push((acc >> 8) as u8);
        }
        if ch.len() > 3 {
            out.push(acc as u8);
        }
    }
    out
}
