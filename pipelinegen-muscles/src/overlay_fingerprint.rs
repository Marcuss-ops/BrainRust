//! Canonical overlay-plan fingerprint (P1.4).
//!
//! Deterministic FNV-1a 64 hash over canonical JSON (object keys sorted
//! recursively, no whitespace). Dependency-free: the crate has no sha2,
//! and the fingerprint only needs stability + collision resistance for
//! cache-skip decisions, not cryptographic strength.
//!
//! The Go mirror (`internal/capabilities/overlays/plan_fingerprint.go`)
//! implements the same canonicalization + FNV-1a; shared vectors in
//! `overlay_fingerprint` tests pin both sides.

use crate::error::MuscleError;

/// FNV-1a 64 offset basis and prime.
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// Hash bytes with FNV-1a 64.
pub fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Render the hash as 16 lowercase hex chars.
pub fn fnv1a64_hex(data: &[u8]) -> String {
    format!("{:016x}", fnv1a64(data))
}

/// Canonicalize a JSON value: object keys sorted recursively, arrays kept
/// in order, numbers emitted in their shortest round-trip form.
pub fn canonical_json(value: &serde_json::Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(true) => out.push_str("true"),
        serde_json::Value::Bool(false) => out.push_str("false"),
        serde_json::Value::Number(n) => out.push_str(&n.to_string()),
        serde_json::Value::String(s) => {
            out.push('"');
            for ch in s.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 => {
                        out.push_str(&format!("\\u{:04x}", c as u32));
                    }
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(&serde_json::Value::String((*key).clone()), out);
                out.push(':');
                write_canonical(&map[*key], out);
            }
            out.push('}');
        }
    }
}

/// Fingerprint a plan JSON document: parse, canonicalize, FNV-1a hex.
/// Fails closed on invalid JSON.
pub fn fingerprint_plan_json(raw: &str) -> Result<String, MuscleError> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| MuscleError::generic(format!("fingerprint: invalid plan JSON: {e}")))?;
    Ok(fnv1a64_hex(canonical_json(&value).as_bytes()))
}

/// Fingerprint an already-parsed plan value.
pub fn fingerprint_plan(value: &serde_json::Value) -> String {
    fnv1a64_hex(canonical_json(value).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_order_does_not_change_fingerprint() {
        let a = serde_json::json!({"b": 1, "a": [1, 2, {"z": true, "m": "x"}]});
        let b = serde_json::json!({"a": [1, 2, {"m": "x", "z": true}], "b": 1});
        assert_eq!(fingerprint_plan(&a), fingerprint_plan(&b));
    }

    #[test]
    fn array_order_matters() {
        let a = serde_json::json!({"clips": ["a", "b"]});
        let b = serde_json::json!({"clips": ["b", "a"]});
        assert_ne!(fingerprint_plan(&a), fingerprint_plan(&b));
    }

    #[test]
    fn shared_vectors_with_go_mirror() {
        // Pinned vectors — the Go mirror must emit the same hashes.
        let cases = [
            ("{\"a\":1}", "fingerprint-vector-a1"),
            (
                "{\"scene_id\":\"s-01\",\"preset\":\"dolly\",\"asset\":\"drive:abc\"}",
                "fingerprint-vector-plan",
            ),
            (
                "{\"b\":[1,2],\"a\":{\"z\":true}}",
                "fingerprint-vector-nested",
            ),
        ];
        for (raw, label) in cases {
            let hash = fingerprint_plan_json(raw).expect("valid fixture JSON");
            assert_eq!(hash.len(), 16, "{label} must be 16 hex chars");
            assert!(
                hash.chars().all(|c| c.is_ascii_hexdigit()),
                "{label} must be hex"
            );
            // Deterministic across calls.
            assert_eq!(
                hash,
                fingerprint_plan_json(raw).unwrap(),
                "{label} unstable"
            );
        }
        // Empty object is a valid (degenerate) plan.
        assert_eq!(fingerprint_plan_json("{}").unwrap().len(), 16);
        // Exact cross-language vectors (Go mirror must match byte-for-byte).
        assert_eq!(
            fingerprint_plan_json("{\"a\":1}").unwrap(),
            "9c3e82dd6fcae8b1"
        );
        assert_eq!(
            fingerprint_plan_json(
                "{\"scene_id\":\"s-01\",\"preset\":\"dolly\",\"asset\":\"drive:abc\"}"
            )
            .unwrap(),
            "472864c08fa1fe6d"
        );
        assert_eq!(
            fingerprint_plan_json("{\"b\":[1,2],\"a\":{\"z\":true}}").unwrap(),
            "156681ee96efe935"
        );
        assert_eq!(fingerprint_plan_json("{}").unwrap(), "08f44b07b5901a25");
    }

    #[test]
    fn invalid_json_fails_closed() {
        assert!(fingerprint_plan_json("not json").is_err());
        assert!(fingerprint_plan_json("").is_err());
    }

    #[test]
    fn timing_state_excluded_by_caller_contract() {
        // The worker strips volatile keys before hashing; the lib hashes
        // exactly what it is given, so different inputs hash differently.
        let with_timing = serde_json::json!({"a": 1, "timing_state": "frozen"});
        let without = serde_json::json!({"a": 1});
        assert_ne!(fingerprint_plan(&with_timing), fingerprint_plan(&without));
    }
}
