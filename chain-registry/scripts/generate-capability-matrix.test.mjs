import assert from "node:assert/strict";
import {
  generateKeyPairSync,
  sign as signMessage,
} from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  afterEach,
  test,
} from "node:test";

import {
  buildCapabilityMatrix,
  assertReleaseReady,
  CR07_MANIFEST_PATH,
  MATRIX_INPUT_PATHS,
  REGISTRY_CONTRACT_PATH,
  SEPOLIA_CANONICAL_SIGNATURE_PATH,
  SEPOLIA_CANONICAL_SPEC_PATH,
  SEPOLIA_PUBLICATION_MIRRORS,
  serializeCapabilityMatrix,
  WEAK_SUBJECTIVITY_TEMPLATE_PATH,
} from "./generate-capability-matrix.mjs";
import {
  canonicalJson,
  computeGenesisHash,
} from "./validate-cr07-activation.mjs";

const SOURCE_ROOT = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "..",
);
const fixtures = [];

function makeFixture() {
  const root = mkdtempSync(
    join(tmpdir(), "creg-capability-matrix-"),
  );
  fixtures.push(root);
  for (const path of MATRIX_INPUT_PATHS) {
    const destination = resolve(root, path);
    mkdirSync(dirname(destination), { recursive: true });
    copyFileSync(resolve(SOURCE_ROOT, path), destination);
  }
  return root;
}

function editText(root, path, mutate) {
  const absolute = resolve(root, path);
  const before = readFileSync(absolute, "utf8");
  const after = mutate(before);
  assert.notEqual(after, before, `fixture mutation changed ${path}`);
  writeFileSync(absolute, after, "utf8");
}

function editJson(root, path, mutate) {
  const absolute = resolve(root, path);
  const value = JSON.parse(readFileSync(absolute, "utf8"));
  mutate(value);
  writeFileSync(
    absolute,
    `${JSON.stringify(value, null, 2)}\n`,
    "utf8",
  );
}

function makePublicationReady(root) {
  const { publicKey, privateKey } =
    generateKeyPairSync("ed25519");
  const publicDer = publicKey.export({
    type: "spki",
    format: "der",
  });
  const publicKeyHex = publicDer.subarray(-32).toString("hex");
  const canonicalPath = resolve(
    root,
    SEPOLIA_CANONICAL_SPEC_PATH,
  );
  const spec = JSON.parse(readFileSync(canonicalPath, "utf8"));
  spec.signing.signing_key_pubkey_hex = publicKeyHex;
  spec.genesis_hash = computeGenesisHash(spec);
  const specBytes = `${JSON.stringify(spec, null, 2)}\n`;
  const signature = signMessage(
    null,
    Buffer.from(
      `creg-chain-spec-v1|${canonicalJson(spec)}`,
      "utf8",
    ),
    privateKey,
  ).toString("hex");
  writeFileSync(canonicalPath, specBytes, "utf8");
  writeFileSync(
    resolve(root, SEPOLIA_CANONICAL_SIGNATURE_PATH),
    signature,
    "utf8",
  );
  for (const mirror of SEPOLIA_PUBLICATION_MIRRORS) {
    writeFileSync(
      resolve(root, mirror.spec_path),
      specBytes,
      "utf8",
    );
    writeFileSync(
      resolve(root, mirror.signature_path),
      signature,
      "utf8",
    );
  }
}

afterEach(() => {
  while (fixtures.length > 0) {
    rmSync(fixtures.pop(), { recursive: true, force: true });
  }
});

test("emits a deterministic repository-only capability matrix", () => {
  const root = makeFixture();
  const first = serializeCapabilityMatrix(
    buildCapabilityMatrix({ root }),
  );
  const second = serializeCapabilityMatrix(
    buildCapabilityMatrix({ root }),
  );
  assert.equal(first, second);

  const matrix = JSON.parse(first);
  assert.equal(matrix.scope.deployed_state, "not_evaluated");
  assert.equal(
    matrix.scope.signed_checkpoint_publication,
    "not_evaluated",
  );
  assert.equal(matrix.chain_specs.length, 6);
  assert.equal(matrix.rollout_defaults.length, 14);
  assert.deepEqual(
    matrix.capabilities.map((capability) => [
      capability.id,
      capability.repository_state,
    ]),
    [
      ["zk_package_qualification", "retired"],
      ["legacy_batch_checkpointing", "retired"],
      ["cr07_authoritative_namespace", "disabled"],
      [
        "weak_subjectivity_checkpoint",
        "staged_unconfigured_defaults",
      ],
      [
        "certified_committee_mandatory_mode",
        "staged_compatibility_defaults",
      ],
      ["signed_sepolia_spec_publication", "blocked"],
    ],
  );
  assert.deepEqual(matrix.release_readiness, {
    status: "blocked",
    ready: false,
    scope: "signed_sepolia_repository_publication_only",
    blockers: [
      "sepolia_canonical_integrity_invalid",
      "sepolia_publication_mirrors_diverge",
      "sepolia_signing_key_unacceptable",
    ],
    complete_system_release_readiness: "not_evaluated",
  });
  const publication = matrix.sepolia_publication_integrity;
  assert.equal(publication.canonical.genesis_hash_valid, false);
  assert.equal(
    publication.canonical.signature_cryptographically_valid,
    false,
  );
  assert.equal(publication.mirrors_exact, false);
  assert.equal(
    publication.mirrors.every(
      (mirror) =>
        mirror.exact_spec_bytes === false &&
        mirror.exact_signature_bytes === false &&
        mirror.status === "blocked",
    ),
    true,
  );
  assert.equal(
    publication.signing_key
      .repository_disclosed_development_key,
    true,
  );
  assert.equal(
    publication.signing_key
      .disallowed_by_activation_manifest,
    true,
  );
  assert.equal(
    publication.signing_key.acceptable_for_publication,
    false,
  );
});

test("release-readiness enforcement rejects the faithfully reported blockers", () => {
  const root = makeFixture();
  const matrix = buildCapabilityMatrix({ root });
  assert.throws(
    () => assertReleaseReady(matrix),
    /sepolia_canonical_integrity_invalid, sepolia_publication_mirrors_diverge, sepolia_signing_key_unacceptable/u,
  );
});

test("release-readiness enforcement rejects a forged ready summary", () => {
  const root = makeFixture();
  const matrix = buildCapabilityMatrix({ root });
  matrix.release_readiness = {
    ...matrix.release_readiness,
    status: "ready",
    ready: true,
    blockers: [],
  };
  assert.throws(
    () => assertReleaseReady(matrix),
    /summary is inconsistent with Sepolia publication integrity facts/u,
  );
});

test("CLI release-readiness mode exits non-zero without emitting a false ready artifact", () => {
  const root = makeFixture();
  const result = spawnSync(
    process.execPath,
    [
      resolve(root, "scripts/generate-capability-matrix.mjs"),
      "--root",
      root,
      "--require-release-ready",
    ],
    { encoding: "utf8" },
  );
  assert.equal(result.status, 1);
  assert.equal(result.stdout, "");
  assert.match(
    result.stderr,
    /release readiness is blocked for signed_sepolia_repository_publication_only/u,
  );
});

test("release-readiness enforcement accepts a fully consistent ephemeral publication fixture", () => {
  const root = makeFixture();
  makePublicationReady(root);
  const matrix = buildCapabilityMatrix({ root });
  assert.equal(matrix.release_readiness.ready, true);
  assert.equal(
    matrix.sepolia_publication_integrity.canonical
      .genesis_hash_valid,
    true,
  );
  assert.equal(
    matrix.sepolia_publication_integrity.canonical
      .signature_cryptographically_valid,
    true,
  );
  assert.equal(
    matrix.sepolia_publication_integrity.mirrors_exact,
    true,
  );
  assert.equal(
    matrix.sepolia_publication_integrity.signing_key
      .acceptable_for_publication,
    true,
  );
  assert.doesNotThrow(() => assertReleaseReady(matrix));
});

test("reports spec and signature equality independently for each mirror", () => {
  const root = makeFixture();
  const [firstMirror] = SEPOLIA_PUBLICATION_MIRRORS;
  copyFileSync(
    resolve(root, SEPOLIA_CANONICAL_SPEC_PATH),
    resolve(root, firstMirror.spec_path),
  );
  copyFileSync(
    resolve(root, SEPOLIA_CANONICAL_SIGNATURE_PATH),
    resolve(root, firstMirror.signature_path),
  );
  const publication =
    buildCapabilityMatrix({ root }).sepolia_publication_integrity;
  assert.equal(publication.mirrors[0].exact_spec_bytes, true);
  assert.equal(
    publication.mirrors[0].exact_signature_bytes,
    true,
  );
  assert.equal(publication.mirrors[0].status, "valid");
  assert.equal(publication.mirrors[1].exact_spec_bytes, false);
  assert.equal(
    publication.mirrors[1].exact_signature_bytes,
    false,
  );
  assert.equal(publication.mirrors_exact, false);
});

test("correct genesis alone remains blocked by the invalid signature", () => {
  const root = makeFixture();
  editJson(root, SEPOLIA_CANONICAL_SPEC_PATH, (spec) => {
    spec.genesis_hash = computeGenesisHash(spec);
  });
  const canonical =
    buildCapabilityMatrix({ root }).sepolia_publication_integrity
      .canonical;
  assert.equal(canonical.genesis_hash_valid, true);
  assert.equal(canonical.signature_cryptographically_valid, false);
  assert.equal(canonical.status, "blocked");
});

test("a disclosed development key stays blocked if removed from only one denylist", () => {
  const root = makeFixture();
  editJson(root, CR07_MANIFEST_PATH, (manifest) => {
    manifest.disallowed_signing_pubkeys = [];
  });
  const signingKey =
    buildCapabilityMatrix({ root }).sepolia_publication_integrity
      .signing_key;
  assert.equal(
    signingKey.repository_disclosed_development_key,
    true,
  );
  assert.equal(
    signingKey.disallowed_by_activation_manifest,
    false,
  );
  assert.equal(signingKey.acceptable_for_publication, false);
});

test("rejects a shipped chain spec that enables retired ZK qualification", () => {
  const root = makeFixture();
  editJson(root, "testnet/chain-spec.local.json", (spec) => {
    spec.feature_flags.zk_validation = true;
  });
  assert.throws(
    () => buildCapabilityMatrix({ root }),
    /feature_flags\.zk_validation must remain false/u,
  );
});

test("rejects an unknown legacy batch capability representation", () => {
  const root = makeFixture();
  editJson(root, "testnet/genesis.json", (spec) => {
    spec.feature_flags.legacy_batch_settlement = true;
  });
  assert.throws(
    () => buildCapabilityMatrix({ root }),
    /feature_flags must contain exactly the supported capability schema/u,
  );
});

test("rejects a ZK entrypoint that does anything before reverting", () => {
  const root = makeFixture();
  editText(root, REGISTRY_CONTRACT_PATH, (source) =>
    source.replace(
      "revert ZKQualificationRetired();",
      "zkValidationEnabled = true;\n        revert ZKQualificationRetired();",
    ),
  );
  assert.throws(
    () => buildCapabilityMatrix({ root }),
    /submitPackageWithZKProof\(\) must contain only/u,
  );
});

test("rejects restoration of the legacy batch settlement body", () => {
  const root = makeFixture();
  editText(root, REGISTRY_CONTRACT_PATH, (source) =>
    source.replace(
      "revert LegacyCheckpointingRetired();",
      "latestStateRoot = nextRoot;",
    ),
  );
  assert.throws(
    () => buildCapabilityMatrix({ root }),
    /submitRollupBatch\(\) must contain only/u,
  );
});

test("delegates CR-07 state validation to the signed activation gate", () => {
  const root = makeFixture();
  editJson(root, "testnet/chain-spec.local.json", (spec) => {
    spec.consensus_params.namespace_authority_activation_height = 100;
  });
  assert.throws(
    () => buildCapabilityMatrix({ root }),
    /existing CR-07 activation validator rejected repository state/u,
  );
});

test("rejects removal of a shipped spec from CR-07 manifest control", () => {
  const root = makeFixture();
  editJson(root, CR07_MANIFEST_PATH, (manifest) => {
    manifest.guarded_chain_specs.pop();
  });
  assert.throws(
    () => buildCapabilityMatrix({ root }),
    /guarded_chain_specs must contain exactly/u,
  );
});

test("rejects mandatory certified committees as a shipped default", () => {
  const root = makeFixture();
  editText(
    root,
    "testnet/bootstrap/bootstrap.env.example",
    (source) =>
      source.replace(
        "CREG_REQUIRE_CERTIFIED_COMMITTEES=false",
        "CREG_REQUIRE_CERTIFIED_COMMITTEES=true",
      ),
  );
  assert.throws(
    () => buildCapabilityMatrix({ root }),
    /must keep CREG_REQUIRE_CERTIFIED_COMMITTEES=false/u,
  );
});

test("rejects a checkpoint certificate baked into a rollout manifest", () => {
  const root = makeFixture();
  editText(root, "docker-compose.prod.yml", (source) =>
    source.replace(
      '"${CREG_WEAK_SUBJECTIVITY_CHECKPOINT:-}"',
      "'{\"schema_version\":1,\"checkpoint\":\"unreviewed\"}'",
    ),
  );
  assert.throws(
    () => buildCapabilityMatrix({ root }),
    /must default CREG_WEAK_SUBJECTIVITY_CHECKPOINT to empty/u,
  );
});

test("rejects omission of either staged rollout control", () => {
  const root = makeFixture();
  editText(root, "k8s/01-configmap.yaml", (source) =>
    source.replace(
      /^\s*CREG_REQUIRE_CERTIFIED_COMMITTEES:.*\r?\n/mu,
      "",
    ),
  );
  assert.throws(
    () => buildCapabilityMatrix({ root }),
    /must declare exactly one default for both staged rollout variables/u,
  );
});

test("rejects a deployable-looking weak-subjectivity input template", () => {
  const root = makeFixture();
  editJson(
    root,
    WEAK_SUBJECTIVITY_TEMPLATE_PATH,
    (template) => {
      template.block_height = 42;
    },
  );
  assert.throws(
    () => buildCapabilityMatrix({ root }),
    /must remain an explicitly non-deployable placeholder/u,
  );
});
