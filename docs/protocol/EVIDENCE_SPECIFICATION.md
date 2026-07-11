# Evidence envelope specification

Each evidence object is a signed, content-addressed envelope with:

| Field | Requirement |
|---|---|
| Subject | Exact artifact digest plus source/manifest subjects where available |
| Package | Ecosystem, namespace, version, publisher, ownership claim |
| Predicate | Typed predicate name and schema version |
| Producer | Workload identity, organization, control/infrastructure domain |
| Tooling | Image digest, tool/version, configuration digest, dataset/rule epoch |
| Environment | OS/runtime/sandbox/build environment digest and network policy |
| Timing | Created-at, deadline, expiry, retention class |
| Result | Observation, limitations, disposition, and raw-object digest |
| Signature | DSSE/in-toto-compatible signature and certificate/bundle reference |

Evidence producers never write policy state directly. The policy engine
consumes envelopes and emits a separate signed decision. Advisory LLM output
is non-deterministic evidence and cannot be the sole hard blocker or dispute
authority.

## Completeness

Profiles declare required predicates. Missing, expired, unavailable, or
environment-incompatible evidence is explicit and fails closed for profiles
that require it.
