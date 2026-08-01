use std::env;

fn main() {
    // The node embeds the spec-signing public key at compile time. Tracking the
    // environment variable here is essential: changing the operator's key must
    // invalidate Cargo's fingerprint and rebuild the binary instead of silently
    // reusing an older pinned key.
    println!("cargo:rerun-if-env-changed=CREG_SPEC_SIGNING_PUBKEY");
    println!("cargo:rerun-if-env-changed=CREG_PRODUCTION");

    if let Ok(raw) = env::var("CREG_SPEC_SIGNING_PUBKEY") {
        let key = raw.trim().trim_start_matches("0x");
        if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            panic!("CREG_SPEC_SIGNING_PUBKEY must be exactly 32 bytes of hexadecimal");
        }
        // Normalize the value embedded by rustc so a leading `0x` in an
        // operator environment cannot produce a different binary policy.
        println!("cargo:rustc-env=CREG_SPEC_SIGNING_PUBKEY={key}");
    }
}
