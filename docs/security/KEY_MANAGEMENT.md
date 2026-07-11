# Key management

Use separate hardware/KMS-backed keys for publisher identity, validator
evidence, log append, witness, checkpoint relay, governance, treasury, and
deployment. Workload identities receive short-lived scoped credentials; worker
containers receive no governance or treasury secrets.

Key ceremonies record participants, quorum, device attestation, backup
location, rotation interval, and revocation procedure. Signatures include
domain, chain/contract, epoch, subject, nonce, and expiry. Compromise response
rotates the affected root, publishes a checkpoint/revocation event, invalidates
old metadata, and forces client freshness.

Secrets must not be copied into the repository, logs, fixtures, or public IPFS.
