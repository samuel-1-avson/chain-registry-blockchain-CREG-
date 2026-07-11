# ADR-004: Publisher identity and namespace ownership

- **Context/problem:** claimed EVM stake and Ed25519 signer were not linked; versions lacked namespace ownership.
- **Options:** trust a claimed address; registry-only ownership; dual-domain proof plus upstream/delegation records.
- **Decision:** use versioned dual proof, replay/freshness fields, and explicit ownership lifecycle.
- **Security implications:** wrong-key, wrong-chain, wrong-contract, expired, and namespace takeover attempts fail.
- **Operational implications:** publishers need wallet and signing-key custody procedures.
- **Migration implications:** legacy submissions are compatibility-only and must be re-attested.
- **Rejected alternatives:** inferring ownership from first presence.
- **Revisit conditions:** broad Sigstore/OIDC coverage replaces legacy wallet binding.
