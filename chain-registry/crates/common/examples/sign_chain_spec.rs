// Chain Spec Signature Generator
// Usage:
//   cargo run --example sign_chain_spec -- <spec.json> --key-env SPEC_SIGNING_KEY
//
// Outputs the hex-encoded Ed25519 detached signature to stdout.

use common::chain_spec::{to_jcs_canonical_json, ChainSpec, SPEC_SIGNATURE_DOMAIN};
use ed25519_dalek::{Signer, SigningKey};
use std::fs;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 || args[2] != "--key-env" {
        eprintln!("Usage: sign_chain_spec <spec.json> --key-env <ENVIRONMENT_VARIABLE>");
        std::process::exit(1);
    }
    let environment_variable = &args[3];
    if environment_variable.is_empty()
        || environment_variable.len() > 64
        || !environment_variable
            .bytes()
            .enumerate()
            .all(|(index, byte)| {
                byte.is_ascii_uppercase()
                    || byte == b'_'
                    || (index > 0 && byte.is_ascii_digit())
            })
    {
        anyhow::bail!("invalid signing-key environment-variable name");
    }

    let spec_json = fs::read_to_string(&args[1])?;
    let spec: ChainSpec = serde_json::from_str(&spec_json)?;

    let private_key = std::env::var(environment_variable)
        .map_err(|_| anyhow::anyhow!("{environment_variable} is not set or is not Unicode"))?;
    let privkey_hex = private_key.trim().trim_start_matches("0x");
    let privkey_bytes = hex::decode(privkey_hex)?;
    let signing_key = SigningKey::from_bytes(
        &privkey_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("private key must be 32 bytes"))?,
    );
    let verifying_key_hex = hex::encode(signing_key.verifying_key().as_bytes());
    const DISCLOSED_DEVELOPMENT_PUBKEY: &str =
        "0437e4adac481519cd6ae66907294c40cfcbf0bdeadd47806f6233be4bd5f82d";
    if verifying_key_hex == DISCLOSED_DEVELOPMENT_PUBKEY {
        anyhow::bail!(
            "refusing repository-disclosed development key; rotate the chain-spec signing key"
        );
    }
    if verifying_key_hex
        != spec
            .signing
            .signing_key_pubkey_hex
            .trim_start_matches("0x")
            .to_ascii_lowercase()
    {
        anyhow::bail!("signing key does not match signing.signing_key_pubkey_hex");
    }

    let jcs = to_jcs_canonical_json(&spec)?;
    let message = format!("{}|{}", SPEC_SIGNATURE_DOMAIN, String::from_utf8(jcs)?);
    let signature = signing_key.sign(message.as_bytes());

    println!("{}", hex::encode(signature.to_bytes()));
    Ok(())
}
