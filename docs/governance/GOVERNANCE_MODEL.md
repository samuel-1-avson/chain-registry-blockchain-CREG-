# Governance and emergency controls

Separate protocol configuration, security response, treasury, validator
admission, disputes, checkpoint authority, and contract upgrades. High-risk
actions require unique independent signers, hardware-backed keys, human and
machine-readable payloads, simulation, timelock, expiry, conflict disclosure,
recusal, and retrospective publication.

The paused state permits only narrow emergency revocation, evidence
preservation, key rotation, communication metadata, and an approved unpause
proposal. `submitEmergencyUnpause` prevents the previous permanent-pause
deadlock. Node operators submit, vote, and execute through
`POST /v1/operator/governance/*` using `CREG_OPERATOR_API_KEY` and
`CREG_GOVERNANCE_KEY`; public list routes remain read-only. Governance
changes are not accepted as a substitute for an external security review.
