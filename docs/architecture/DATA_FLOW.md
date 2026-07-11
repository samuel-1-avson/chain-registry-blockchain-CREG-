# Data flow and canonical state

The canonical sequence is:

`submission receipt → ownership event → evidence envelopes → policy decision → witness tree head → checkpoint → derived views → client verification`

The witnessed event log is the target source of truth. Until migration is
complete, the local chain is a legacy event source and RocksDB is its local
projection. PostgreSQL, search, explorer responses, caches, IPFS gateways, and
RPC responses are never authoritative.

## Event requirements

Every event has a schema version, event ID, subject digests, issuer identity,
creation time, expiry/freshness, policy/profile reference where applicable,
previous-event or ownership reference, and signature. Corrections are new
superseding events. Deletes are tombstones; personal/private material is kept
off the public immutable log.

## Availability flow

Evidence required by an active profile is replicated across at least two
administrative domains. Retrieval probes record availability. If evidence is
missing, the profile expires or downgrades and the client receives a bounded
failure, while the digest and history remain auditable.

## Migration invariant

The target projection must be reproducible from exported legacy blocks and
receipts. A migration tool must report every record that cannot be mapped; it
must never silently drop or reinterpret a package decision.
