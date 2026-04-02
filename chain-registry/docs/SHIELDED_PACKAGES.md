# Shielded (Encrypted) Packages

## Overview

Shielded packages provide **confidentiality** for sensitive software packages. This feature is essential for:

- **Enterprise private registries** - Companies publishing proprietary code
- **Trade secrets** - Algorithms and business logic protection
- **Internal APIs** - Confidential service interfaces
- **Security research** - Vulnerability disclosure packages

## How It Works

### The Problem

Traditional blockchain registries are **fully transparent**. Anyone can:
- See package content
- Download and inspect code
- Fork and copy proprietary logic

This prevents enterprises from using blockchain registries for sensitive code.

### The Solution: Threshold Encryption

```
┌─────────────────────────────────────────────────────────────────────┐
│                    SHIELDED PACKAGE ARCHITECTURE                     │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  PUBLISHING (Enterprise)                                            │
│  ───────────────────────                                            │
│  1. Package: "company/internal-api@1.0.0"                          │
│  2. Generate random encryption key                                 │
│  3. Encrypt package with AES-256-GCM                               │
│  4. Split key into shares (Shamir's Secret Sharing)                │
│  5. Encrypt shares to validator public keys                        │
│  6. Upload encrypted package to IPFS                               │
│  7. Submit to chain: metadata + encrypted shares                   │
│                                                                      │
│                              ↓                                       │
│                                                                      │
│  BLOCKCHAIN (Public)                                                │
│  ───────────────────                                                │
│  Store:  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐    │
│          │   Package   │  │   Encrypted │  │  Key Shares     │    │
│          │   Metadata  │  │   IPFS CID  │  │  (1 per validator)│   │
│          │   (public)  │  │   (public)  │  │  (encrypted)    │    │
│          └─────────────┘  └─────────────┘  └─────────────────┘    │
│                                                                      │
│  Don't Store: Package content, encryption key                       │
│                                                                      │
│                              ↓                                       │
│                                                                      │
│  INSTALLATION (Authorized User)                                     │
│  ──────────────────────────────                                     │
│  1. Request package installation                                   │
│  2. Validators verify authorization                                │
│  3. Validators decrypt their shares                                │
│  4. M-of-N validators collaborate to reconstruct key               │
│  5. Package decrypted and delivered                                │
│  6. Unauthorized parties cannot decrypt                            │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

## Technical Details

### Encryption Scheme

**Hybrid Encryption:**
1. **Random AES-256-GCM key** encrypts the package
2. **Shamir's Secret Sharing (SSS)** splits the key into N shares
3. **Threshold M** required to reconstruct the key
4. **RSA/ECIES** encrypts each share to validator's public key

### Security Properties

| Threat | Protection |
|--------|------------|
| Single validator compromised | Can't decrypt (needs M shares) |
| Blockchain public | Only metadata visible |
| IPFS public | Content encrypted |
| Unauthorized user | Can't get shares |
| Package publisher | Can revoke access |

## Usage

### Publishing a Shielded Package

```bash
# Normal publish (public)
creg publish ./my-package.tgz

# Shielded publish (encrypted)
creg publish ./my-package.tgz --shield

# Shielded with access control
creg publish ./my-package.tgz --shield \
  --allow-user alice@company.com \
  --allow-user bob@company.com \
  --organization company-internal
```

### Installing a Shielded Package

```bash
# Normal install (works for public)
creg install company/public-package

# Shielded install (requires authorization)
creg install company/internal-api --key ~/.creg/company-key.pem

# Or with identity verification
creg install company/internal-api --identity-verification
```

### Managing Access

```bash
# Grant access to a user
creg shield grant company/internal-api@1.0.0 user@company.com

# Revoke access
creg shield revoke company/internal-api@1.0.0 user@company.com

# List authorized users
creg shield list-access company/internal-api@1.0.0

# Rotate encryption key
creg shield rotate-key company/internal-api@1.0.0
```

## Implementation

### Components

```
crates/threshold-encryption/
├── src/
│   ├── lib.rs              # Main types and exports
│   ├── shamir.rs           # Shamir's Secret Sharing
│   ├── distribution.rs     # Key share distribution
│   ├── service.rs          # Decryption service (per validator)
│   └── access_control.rs   # Authorization policies
```

### Key Share Distribution

```rust
use threshold_encryption::{ShareDistributor, ShieldedPackageMetadata};

// Create distributor for 3-of-5 threshold
let mut distributor = ShareDistributor::new(3, 5)?;

// Register validators
distributor.register_validator("val1".to_string(), pubkey1);
distributor.register_validator("val2".to_string(), pubkey2);
// ...

// Distribute shares
let shares = distributor.distribute_shares(
    "npm:@company/internal@1.0.0",
    &encryption_key,
    &access_policy,
)?;
```

### Decryption Service

Each validator runs a decryption service:

```rust
use threshold_encryption::{DecryptionService, ServiceConfig};

let config = ServiceConfig {
    validator_id: "val1".to_string(),
    validator_key: my_private_key,
    threshold: 3,
    total_shares: 5,
};

let service = DecryptionService::new(config, rx, tx)?;
service.run().await;
```

### Decryption Flow

```rust
// Client requests decryption
let client = DecryptionClient::new(cmd_tx, resp_rx, my_key, my_pubkey);
let responses = client.request_decryption(
    "npm:@company/internal@1.0.0",
    "Production deployment"
).await?;

// Reconstruct package
let package = client.reconstruct_package(
    &encrypted_data,
    &responses
)?;
```

## Access Control

### Policy Types

1. **Organization-Only**
   ```json
   {
     "organization_only": true,
     "organization_id": "company-internal"
   }
   ```

2. **Specific Users**
   ```json
   {
     "authorized_decryptors": [
       "alice@company.com",
       "bob@company.com"
     ]
   }
   ```

3. **Time-Limited**
   ```json
   {
     "expires_at": 1700000000,
     "max_decryptions": 100
   }
   ```

### Authorization Check Flow

```
Decryption Request
      ↓
Check User Identity
      ↓
Check Organization Membership (if required)
      ↓
Check Time Limits
      ↓
Check Decryption Quota
      ↓
Valid? → Collect Shares
Invalid? → Reject
```

## Economic Model

### Costs

| Operation | Cost | Reason |
|-----------|------|--------|
| Shielded Publish | 10 CREG | Higher storage for encrypted shares |
| Decryption Request | 1 CREG | Validator coordination overhead |
| Access Grant | 0.1 CREG | State update |
| Key Rotation | 5 CREG | Re-encryption of shares |

### Validator Rewards

Validators earn extra rewards for participating in decryption:
- **Base reward:** 0.1 CREG per decryption share provided
- **Coordination bonus:** 0.5 CREG for completing M-of-N consensus

## Security Considerations

### Threat Model

| Attacker | Capability | Defense |
|----------|------------|---------|
| Single validator | Can see one share | Needs M-1 more shares |
| Blockchain observer | Sees encrypted data | Encryption |
| Malicious user | Requests decryption | Authorization checks |
| Compromised IPFS | Accesses encrypted content | Client-side encryption |

### Best Practices

1. **Use high threshold** (e.g., 5-of-9) for critical packages
2. **Rotate keys regularly** for long-lived packages
3. **Audit access logs** for unauthorized decryption attempts
4. **Use separate validator sets** for public vs private packages
5. **Time-limit access** when possible

## Comparison

### With vs Without Shielded Packages

| Feature | Public Only | With Shielded |
|---------|-------------|---------------|
| Open source packages | ✅ | ✅ |
| Proprietary packages | ❌ | ✅ |
| Enterprise adoption | Limited | High |
| Verification transparency | ✅ | ✅ |
| Content confidentiality | ❌ | ✅ |
| Cost per package | Lower | Higher |

### vs Traditional Private Registries

| Feature | npm Private | Shielded Chain |
|---------|-------------|----------------|
| Decentralized | ❌ | ✅ |
| Immutable history | ❌ | ✅ |
| Multi-party verification | ❌ | ✅ |
| Cryptographic access control | ❌ | ✅ |
| Censorship resistance | ❌ | ✅ |
| Enterprise SSO | ✅ | ✅ |

## Future Enhancements

### Phase 1 (Current)
- ✅ Basic threshold encryption
- ✅ M-of-N consensus
- ✅ Access control policies

### Phase 2 (Planned)
- **Attribute-based encryption** - decrypt based on roles
- **Time-locked decryption** - decrypt after specific date
- **Revocable decryption** - invalidate past decryptions

### Phase 3 (Advanced)
- **Zero-knowledge proofs** - prove authorization without revealing identity
- **Homomorphic encryption** - validate encrypted packages
- **Secure multi-party computation** - distributed validation

## Troubleshooting

### "Insufficient shares for decryption"
- Not enough validators are online
- Check validator participation rates
- Consider lowering threshold for critical packages

### "Access denied"
- Requestor not in authorized list
- Organization membership expired
- Decryption quota exceeded

### "Key reconstruction failed"
- Corrupted shares during transmission
- Mismatch in threshold parameters
- Contact package publisher for re-keying

## API Reference

### Smart Contract

```solidity
// Store encrypted shares
function storeEncryptedShares(
    bytes32 packageId,
    bytes[] calldata encryptedShares,
    uint8 threshold,
    AccessPolicy calldata policy
) external;

// Request decryption (triggers M-of-N consensus)
function requestDecryption(
    bytes32 packageId,
    bytes calldata authorizationProof
) external returns (uint256 requestId);

// Submit decryption share (called by validators)
function submitDecryptionShare(
    uint256 requestId,
    bytes calldata encryptedShare
) external;
```

### Rust Library

```rust
// Distribute shares
let distributor = ShareDistributor::new(3, 5)?;
let shares = distributor.distribute_shares(canonical, key, policy)?;

// Decrypt
let client = DecryptionClient::new(cmd_tx, resp_rx, key, pubkey);
let responses = client.request_decryption(canonical, purpose).await?;
let package = client.reconstruct_package(&encrypted, &responses)?;
```

---

*Last updated: 2026-04-02*
