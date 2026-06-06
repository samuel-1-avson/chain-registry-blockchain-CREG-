# Prompt improvement tips — Chain Registry (from session analysis)

Derived from high-sufficiency turns in the Cursor remediation session and clean Antigravity Docker-ready session. Use these when steering the agent on this repo.

---

## 1. Audit or research (one shot)

```
Analyze the chain-registry monorepo and write docs/<NAME>.md covering:
- architecture, repo structure, workflows
- API, database, blockchain, wallet subsystems
- readiness score, known weaknesses, recommended fixes

Save as markdown only. Do not change application code unless I ask.
```

**Why it worked:** Clear deliverable, explicit exclusions, section list.

---

## 2. Plan-backed execution slice

```
Work item: SEC-301b
Done when:
- chain-registry-secrets crate with env + Vault backends
- node, faucet, relayer wired
- cargo test -p chain-registry-secrets passes
- docs/REMEDIATION_BACKLOG.md updated

Exclude from commit: sepolia-node-data/, target/, testnet/spec-server/*.json
Do not: Sepolia deploy, unrelated refactors
```

**Why it works:** Bounded scope, test gate, commit hygiene.

---

## 3. Windows-friendly verification

```
Verify SEC-301b only:
1. cargo test -p chain-registry-secrets
2. cargo check -p faucet -p chain-registry-relayer
Do not run full workspace test unless the above pass.
Report timings.
```

**Why it works:** Avoids 15+ minute full node builds unless needed.

---

## 4. Steering without scope explosion

Instead of: `go` / `start` / `what is next`

Use:

```
Continue per SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md Phase 3 epic 3.4 —
next item only: SEC-301a (KMS ADR). No Sepolia ops this turn.
```

---

## 5. Ops vs code separation

```
OPS thread: Run Sepolia Option A reuse path per TESTNET_SEPOLIA_RUNBOOK.md.
Report: validator_set_sync status, spec URL, blockers.
No Rust feature work.

---

DELIVERY thread: Implement SEC-203 alloy unification only.
```

---

## 6. Phase closure gate

```
Close Phase 2 per PHASE2_CLOSEOUT.md:
- confirm REM-203 merged
- second-operator checklist stub only (no deploy)
Then stop; do not start Phase 3 until I say "Phase 3".
```

---

## Anti-patterns observed

| Prompt | Effect |
|:---|:---|
| `go` (alone) | Agent picks entire Phase 1 backlog |
| `what is next` (repeated) | Re-plans instead of executing one item |
| `start multitasking` + long compiles | User perceives stall; lock contention |
| Mixing Sepolia + SEC-203 + Phase 3 in one turn | Context switching, uncommitted diffs |
