use anyhow::{bail, Context, Result};
use common::crypto::SigningKey;

#[derive(Debug, Default)]
struct Args {
    chain_id: String,
    evm_address: String,
    node_id: String,
    validator_key_hex: String,
    validator_pq_key_hex: Option<String>,
    nonce: String,
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let key_array = decode_seed(&args.validator_key_hex, "validator key")?;
    let signing_key = SigningKey::from_ed25519_seed(&key_array);
    let ed25519_pubkey = signing_key.public_key().to_hex();

    let message = validator_identity_registration_message(
        &args.chain_id,
        &args.evm_address.to_ascii_lowercase(),
        &args.node_id,
        &ed25519_pubkey,
        &args.nonce,
    );
    let signature = signing_key.sign(message.as_bytes());

    let mut output = serde_json::json!({
        "message": message,
        "ed25519_pubkey": ed25519_pubkey,
        "ed25519_signature": signature.to_hex(),
    });

    // ADR-017: with the ML-DSA-44 seed, also prove possession of the
    // post-quantum key so the node can bind it to this validator.
    if let Some(pq_key_hex) = &args.validator_pq_key_hex {
        let pq_seed = decode_seed(pq_key_hex, "validator ML-DSA-44 seed")?;
        let hybrid_key = SigningKey::hybrid_from_seeds(&key_array, &pq_seed);
        let binding = common::PqKeyBindingV1::sign_with(
            args.node_id.as_str(),
            args.chain_id.as_str(),
            &hybrid_key,
        )?;
        output["pq_pubkey"] = serde_json::json!(binding.pq_pubkey);
        output["pq_binding_signature"] = serde_json::json!(binding.compact_signature());
    }

    println!("{output}");

    Ok(())
}

fn decode_seed(raw: &str, label: &str) -> Result<[u8; 32]> {
    let trimmed = raw.strip_prefix("0x").unwrap_or(raw);
    let bytes = hex::decode(trimmed).with_context(|| format!("{label} must be hex"))?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("{label} must be exactly 32 bytes"))
}

fn validator_identity_registration_message(
    chain_id: &str,
    evm_address: &str,
    node_id: &str,
    ed25519_pubkey: &str,
    nonce: &str,
) -> String {
    format!(
        "creg-validator-identity-v1\nchain_id:{chain_id}\nevm_address:{evm_address}\nnode_id:{node_id}\ned25519_pubkey:{ed25519_pubkey}\nnonce:{nonce}"
    )
}

fn parse_args() -> Result<Args> {
    let mut parsed = Args::default();
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--chain-id" => parsed.chain_id = required_value(&mut args, "--chain-id")?,
            "--evm-address" => parsed.evm_address = required_value(&mut args, "--evm-address")?,
            "--node-id" => parsed.node_id = required_value(&mut args, "--node-id")?,
            "--validator-key-hex" => {
                parsed.validator_key_hex = required_value(&mut args, "--validator-key-hex")?
            }
            "--validator-pq-key-hex" => {
                parsed.validator_pq_key_hex =
                    Some(required_value(&mut args, "--validator-pq-key-hex")?)
            }
            "--nonce" => parsed.nonce = required_value(&mut args, "--nonce")?,
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    if parsed.chain_id.is_empty()
        || parsed.evm_address.is_empty()
        || parsed.node_id.is_empty()
        || parsed.validator_key_hex.is_empty()
        || parsed.nonce.is_empty()
    {
        print_usage();
        bail!("missing required argument");
    }

    Ok(parsed)
}

fn required_value(args: &mut impl Iterator<Item = String>, name: &str) -> Result<String> {
    args.next()
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("{name} requires a value"))
}

fn print_usage() {
    eprintln!(
        "Usage: cargo run -p chain-registry-cli --example sign_validator_identity -- \\
  --chain-id creg-testnet-1 \\
  --evm-address 0x... \\
  --node-id validator-1 \\
  --validator-key-hex <32-byte-ed25519-private-key-hex> \\
  [--validator-pq-key-hex <32-byte-ml-dsa-44-seed-hex>] \\
  --nonce <unique-nonce>

With --validator-pq-key-hex the output also carries pq_pubkey and
pq_binding_signature (ADR-017 creg-pq-binding-v1) for the registration."
    );
}
