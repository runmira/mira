//! PKCE (RFC 7636) primitives shared by every OAuth provider flow.
//!
//! Both OpenRouter and OpenAI/Codex use identical S256 challenge
//! semantics, so this module carries the reusable half. The
//! provider-specific code (endpoint URLs, param names) lives in the
//! provider submodule.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Generate a fresh PKCE code_verifier. Returns a URL-safe, unpadded
/// base64 string in the RFC-7636-approved 43..128-character range.
///
/// 64 random bytes → 86 base64url characters, comfortably inside the
/// allowed length window and well past the 256-bit entropy floor most
/// providers require.
pub fn gen_verifier() -> String {
    let mut buf = [0u8; 64];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Compute the S256 code_challenge for a verifier — SHA-256 of the
/// verifier, base64url-encoded without padding. Providers compare the
/// result against the challenge they received in the authorize request.
pub fn code_challenge_s256(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// Generate an opaque flow_id used to correlate the browser round-trip.
/// Passed as `state` on the authorize URL and expected back verbatim on
/// the callback — the CSRF guard.
///
/// Distinct from the verifier by design: leaking `state` in a browser
/// history / referrer log doesn't leak the PKCE secret.
pub fn gen_flow_id() -> String {
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_length_and_charset() {
        for _ in 0..10 {
            let v = gen_verifier();
            assert!(
                (43..=128).contains(&v.len()),
                "verifier length {} outside RFC 7636 range",
                v.len()
            );
            assert!(
                v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "verifier contains non-URL-safe char: {v}"
            );
        }
    }

    #[test]
    fn challenge_is_deterministic_and_matches_known_vector() {
        // RFC 7636 Appendix B — a known verifier/challenge pair.
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            code_challenge_s256(v),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn flow_id_is_unique() {
        let a = gen_flow_id();
        let b = gen_flow_id();
        assert_ne!(a, b, "flow ids collided across two calls");
    }
}
