#!/usr/bin/env python3
"""
generate-testnet-keys.py

Generates Ed25519 validator keys for the Chain Registry testnet and produces:
  - .env.testnet         : Docker Compose environment variables
  - config/validator-set.json : The CREG_VALIDATOR_SET value

The local Docker bootstrap flow runs one validator node on this machine.

Usage:
  python scripts/generate-testnet-keys.py --nodes 1 --output .env.testnet
"""

import argparse
import json
import os
import sys
from pathlib import Path

# Try multiple Ed25519 backends so the script works in most environments.
def _try_ed25519_backends():
    backends = []
    
    # Backend 1: cryptography (most common)
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        def crypt_backend(seed: bytes) -> tuple[str, str]:
            sk = Ed25519PrivateKey.from_private_bytes(seed)
            pk = sk.public_key()
            return (
                seed.hex(),
                pk.public_bytes_raw().hex()
            )
        backends.append(("cryptography", crypt_backend))
    except Exception:
        pass

    # Backend 2: pynacl
    try:
        import nacl.bindings
        def nacl_backend(seed: bytes) -> tuple[str, str]:
            pk, sk = nacl.bindings.crypto_sign_seed_keypair(seed)
            return (seed.hex(), pk.hex())
        backends.append(("pynacl", nacl_backend))
    except Exception:
        pass

    # Backend 3: ed25519 (pure-python, older but common)
    try:
        import ed25519
        def ed25519_backend(seed: bytes) -> tuple[str, str]:
            sk = ed25519.SigningKey(seed)
            pk = sk.get_verifying_key()
            return (seed.hex(), pk.to_ascii(encoding="hex").decode())
        backends.append(("ed25519", ed25519_backend))
    except Exception:
        pass

    if not backends:
        print("ERROR: No Ed25519 library found.")
        print("Install one of: pip install cryptography | pynacl | ed25519")
        sys.exit(1)

    return backends


def generate_keypair(backends, node_id: str) -> tuple[str, str]:
    """Generate a deterministic Ed25519 keypair using the first working backend."""
    import secrets
    seed = secrets.token_bytes(32)
    for name, backend in backends:
        try:
            priv, pub = backend(seed)
            return priv, pub
        except Exception as e:
            print(f"  Backend {name} failed: {e}", file=sys.stderr)
            continue
    print("ERROR: All Ed25519 backends failed.", file=sys.stderr)
    sys.exit(1)


def main():
    parser = argparse.ArgumentParser(
        description="Generate validator keys for Chain Registry testnet"
    )
    parser.add_argument(
        "--nodes", type=int, default=1,
        help="Number of validator node definitions to generate (default: 1)"
    )
    parser.add_argument(
        "--output", type=str, default=".env.testnet",
        help="Output env file (default: .env.testnet)"
    )
    parser.add_argument(
        "--config-dir", type=str, default="config",
        help="Directory for generated config files"
    )
    args = parser.parse_args()

    backends = _try_ed25519_backends()
    print(f"Using Ed25519 backend: {backends[0][0]}")
    print(f"Generating validator definitions for {args.nodes} node(s)...\n")

    os.makedirs(args.config_dir, exist_ok=True)

    env_lines = [
        "# Chain Registry Testnet Environment",
        f"# Generated for {args.nodes} validator node(s)",
        "",
        "# Host-facing endpoints for commands run outside Docker",
        "CREG_ETH_RPC=http://localhost:8545",
        "CREG_IPFS_URL=http://localhost:5001",
        "",
        "# Docker-internal endpoints for containers inside docker-compose.testnet.yml",
        "CREG_DOCKER_ETH_RPC=http://anvil:8545",
        "CREG_DOCKER_IPFS_URL=http://ipfs:5001",
        "CREG_PG_URL=postgres://${POSTGRES_USER:-creg}:${POSTGRES_PASSWORD:-creg}@postgres:5432/${POSTGRES_DB:-chain_registry}",
        "",
    ]

    validators = []
    for i in range(1, args.nodes + 1):
        node_id = f"node-{i}"
        priv, pub = generate_keypair(backends, node_id)
        env_lines.append(f"NODE{i}_VALIDATOR_KEY={priv}")
        validators.append({
            "id": node_id,
            "alias": f"Validator-{i}",
            "pubkey": pub,
            "stake": 100,
            "reputation": 100,
            "status": "online"
        })
        print(f"  {node_id}: pubkey = {pub[:16]}...{pub[-8:]}")

    validator_set_json = json.dumps({"validators": validators})
    env_lines.append(f'\nVALIDATOR_SET_JSON={validator_set_json}')

    # Also generate a publisher key for stress testing
    pub_priv, pub_pub = generate_keypair(backends, "publisher")
    env_lines.append(f"\nTESTNET_PUBLISHER_KEY={pub_priv}")
    env_lines.append(f"TESTNET_PUBLISHER_PUBKEY={pub_pub}")

    # Write env file
    output_path = Path(args.output)
    output_path.write_text("\n".join(env_lines) + "\n")
    print(f"\n[WROTE] {output_path}")

    # Write standalone validator-set.json for reference
    validator_set_path = Path(args.config_dir) / "validator-set.json"
    validator_set_path.write_text(json.dumps({"validators": validators}, indent=2))
    print(f"[WROTE] {validator_set_path}")

    print("\nNext steps:")
    print(f"  1. Review {output_path}")
    print("  2. Deploy the bootstrap host (single validator on this machine):")
    print("     docker compose -f docker-compose.testnet.yml --env-file .env.testnet up -d --build")
    print("  3. Run stress test:")
    print("     python scripts/stress-test.py --nodes 1 --packages 1000")


if __name__ == "__main__":
    main()
