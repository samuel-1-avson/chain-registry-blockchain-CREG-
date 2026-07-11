# Test strategy

Every trust-boundary change requires unit, integration, end-to-end, adversarial,
property, fuzz, and conformance coverage appropriate to the layer.

## Required suites

- Rust: fmt, Clippy warnings denied, workspace tests, dependency/license/unsafe
  policy, property tests, malicious fixtures, and coverage.
- Solidity: pinned dependencies, compile, unit, invariant, fuzz, static,
  gas/deployment, storage layout, pause/recovery, and upgrade tests.
- Web/TypeScript: typecheck, lint, unit, integration, e2e, accessibility,
  dependency audit, and reproducible build.
- Infrastructure: config/policy validation, secrets/container/SBOM scans,
  backup/restore, and disaster recovery.
- Protocol: replay, rollback, freeze, split-view, partition, Byzantine,
  committee sizes 1/4/7/50/51/100/1k/10k, dropout, and key compromise.

Security-test failure blocks release; a skipped or mocked failure is not a fix.
