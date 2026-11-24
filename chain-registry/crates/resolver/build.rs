use std::env;

fn main() {
    // The resolver embeds the spec-signing public key at compile time when set.
    // Track changes here so rotating the pinned key rebuilds resolver crates.
    println!("cargo:rerun-if-env-changed=CREG_SPEC_SIGNING_PUBKEY");

    if let Ok(raw) = env::var("CREG_SPEC_SIGNING_PUBKEY") {
        let key = raw.trim().trim_start_matches("0x");
        if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            panic!("CREG_SPEC_SIGNING_PUBKEY must be exactly 32 bytes of hexadecimal");
        }
        println!("cargo:rustc-env=CREG_SPEC_SIGNING_PUBKEY={key}");
    }
}
