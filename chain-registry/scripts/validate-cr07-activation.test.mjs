import assert from "node:assert/strict";
import {
  generateKeyPairSync,
  sign as signMessage,
} from "node:crypto";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  CHAIN_SPEC_SIGNATURE_DOMAIN,
  activationPayloadDigest,
  canonicalJson,
  computeGenesisHash,
  inspectActivationCandidate,
  sha256Hex,
  validateActivationGate,
} from "./validate-cr07-activation.mjs";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function ephemeralSigningMaterial() {
  // Test-only, generated afresh in the OS temporary directory. No test key or
  // signature is checked in or suitable for an operator trust root.
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const publicDer = publicKey.export({ type: "spki", format: "der" });
  return {
    privateKey,
    publicKeyHex: publicDer.subarray(publicDer.length - 32).toString("hex"),
  };
}

function policy(authority, byte) {
  return {
    authority,
    adapter_id: `${authority}-adapter`,
    adapter_version: "1.0.0",
    verifier_sha256: byte.repeat(64),
    policy_sha256: (byte === "a" ? "b" : "c").repeat(64),
    trust_root_sha256: (byte === "a" ? "d" : "e").repeat(64),
  };
}

function makeSpec(publicKeyHex, { active = true, policies } = {}) {
  const consensusParams = {
    block_time_seconds: 5,
    vote_timeout_ms: 10_000,
    quorum_percentage: 67,
    min_validator_stake_wei: "100",
    min_publisher_stake_wei: "10",
    unbonding_period_seconds: 86_400,
    slash_penalty_low_bp: 200,
    slash_penalty_medium_bp: 1_000,
    slash_penalty_critical_bp: 3_000,
    max_validators: 50,
  };
  if (active) {
    consensusParams.namespace_authority_activation_height = 1_000;
    consensusParams.namespace_authority_policies = policies ?? [
      policy("npm_trusted_publisher", "a"),
      policy("pypi_trusted_publisher", "f"),
    ];
  }

  const spec = {
    spec_version: 1,
    chain_id: "creg-test-fixture",
    network: "testnet",
    phase: "alpha",
    genesis_time: "2026-07-29T00:00:00Z",
    genesis_hash: `0x${"0".repeat(64)}`,
    consensus_params: consensusParams,
    feature_flags: {
      zk_validation: false,
      ml_validation: false,
      wasm_sandbox: true,
      private_registries: false,
      cross_chain: false,
      insurance: false,
      threshold_encryption: false,
    },
    l1: {
      name: "fixture",
      chain_id: 31_337,
      block_explorer: "http://127.0.0.1",
      min_finality_blocks: 1,
    },
    contracts: {
      staking: `0x${"1".repeat(40)}`,
    },
    bootnodes: [],
    validator_set: {
      version: 1,
      last_updated: "2026-07-29T00:00:00Z",
      epoch_block_height: 0,
      validators: [
        {
          id: "validator-1",
          alias: "Fixture Validator",
          pubkey: publicKeyHex,
          eth_address: `0x${"2".repeat(40)}`,
          stake: 100,
          reputation: 100,
          status: "active",
        },
      ],
    },
    services: {
      ipfs_gateway: "http://127.0.0.1",
      ipfs_api: "http://127.0.0.1",
      faucet: "http://127.0.0.1",
      explorer: "http://127.0.0.1",
      metrics: "http://127.0.0.1",
    },
    signing: {
      signature_algorithm: "ed25519",
      signing_key_pubkey_hex: publicKeyHex,
      detached_signature_url: "https://invalid.example/fixture.sig",
    },
  };
  spec.genesis_hash = computeGenesisHash(spec);
  return spec;
}

function signSpec(spec, privateKey) {
  const message = Buffer.from(
    `${CHAIN_SPEC_SIGNATURE_DOMAIN}|${canonicalJson(spec)}`,
    "utf8",
  );
  return signMessage(null, message, privateKey).toString("hex");
}

function signedManifest(spec, { mirrors = [] } = {}) {
  const state = {
    height: spec.consensus_params.namespace_authority_activation_height,
    policies: spec.consensus_params.namespace_authority_policies,
  };
  return {
    manifest_version: 1,
    status: "signed",
    signature_domain: CHAIN_SPEC_SIGNATURE_DOMAIN,
    chain_spec_version: 1,
    required_authorities: [
      "npm_trusted_publisher",
      "pypi_trusted_publisher",
    ],
    disallowed_signing_pubkeys: [],
    guarded_chain_specs: [
      "testnet/activation.json",
      ...mirrors.map((mirror) => mirror.spec_path),
    ],
    activation: {
      spec_path: "testnet/activation.json",
      signature_path: "testnet/activation.json.sig",
      mirrors,
      chain_id: spec.chain_id,
      genesis_hash: spec.genesis_hash,
      signing_pubkey_hex: spec.signing.signing_key_pubkey_hex,
      activation_height:
        spec.consensus_params.namespace_authority_activation_height,
      activation_payload_sha256: activationPayloadDigest(spec, state),
      canonical_spec_sha256: sha256Hex(canonicalJson(spec)),
    },
  };
}

function fixtureDirectory() {
  const root = mkdtempSync(join(tmpdir(), "creg-cr07-gate-"));
  mkdirSync(join(root, "config"), { recursive: true });
  mkdirSync(join(root, "testnet"), { recursive: true });
  return root;
}

function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function writeSignedFixture({ policies, mirrors = [] } = {}) {
  const root = fixtureDirectory();
  const signing = ephemeralSigningMaterial();
  const spec = makeSpec(signing.publicKeyHex, { policies });
  const signature = signSpec(spec, signing.privateKey);
  writeJson(join(root, "testnet", "activation.json"), spec);
  writeFileSync(
    join(root, "testnet", "activation.json.sig"),
    `${signature}\n`,
    "utf8",
  );
  for (const mirror of mirrors) {
    writeJson(join(root, mirror.spec_path), spec);
    writeFileSync(join(root, mirror.signature_path), `${signature}\n`, "utf8");
  }
  const manifest = signedManifest(spec, { mirrors });
  writeJson(join(root, "config", "cr07-activation-manifest.json"), manifest);
  return { root, spec, signature, manifest };
}

test("repository manifest keeps every operational spec disabled", () => {
  const result = validateActivationGate({ root: repositoryRoot });
  assert.equal(result.status, "disabled");
  assert.equal(result.guardedSpecs, 4);
  assert.equal(result.activationSpec, null);
});

test("disabled manifest rejects an unmanifested activation", () => {
  const root = fixtureDirectory();
  try {
    const signing = ephemeralSigningMaterial();
    const spec = makeSpec(signing.publicKeyHex);
    writeJson(join(root, "testnet", "activation.json"), spec);
    writeJson(join(root, "config", "cr07-activation-manifest.json"), {
      manifest_version: 1,
      status: "disabled",
      signature_domain: CHAIN_SPEC_SIGNATURE_DOMAIN,
      chain_spec_version: 1,
      required_authorities: [
        "npm_trusted_publisher",
        "pypi_trusted_publisher",
      ],
      disallowed_signing_pubkeys: [],
      guarded_chain_specs: ["testnet/activation.json"],
      activation: null,
    });

    assert.throws(
      () => validateActivationGate({ root }),
      /activates CR-07 while the repository manifest is disabled/u,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("signed manifest accepts an exact ephemeral spec and mirror", () => {
  const mirror = {
    spec_path: "testnet/mirror/activation.json",
    signature_path: "testnet/mirror/activation.json.sig",
  };
  const fixture = writeSignedFixture({ mirrors: [mirror] });
  try {
    const result = validateActivationGate({ root: fixture.root });
    assert.equal(result.status, "signed");
    assert.equal(result.activationSpec, "testnet/activation.json");
    assert.equal(result.genesisHash, fixture.spec.genesis_hash);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("candidate inspection emits the exact manifest pins", () => {
  const fixture = writeSignedFixture();
  try {
    const candidate = inspectActivationCandidate({
      root: fixture.root,
      specPath: "testnet/activation.json",
    });
    assert.equal(candidate.chain_id, fixture.manifest.activation.chain_id);
    assert.equal(
      candidate.activation_payload_sha256,
      fixture.manifest.activation.activation_payload_sha256,
    );
    assert.equal(
      candidate.canonical_spec_sha256,
      fixture.manifest.activation.canonical_spec_sha256,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("signed manifest rejects a forged detached signature", () => {
  const fixture = writeSignedFixture();
  try {
    const signaturePath = join(
      fixture.root,
      "testnet",
      "activation.json.sig",
    );
    const signature = readFileSync(signaturePath, "utf8").trim();
    const replacement = signature.startsWith("0") ? "1" : "0";
    writeFileSync(
      signaturePath,
      `${replacement}${signature.slice(1)}\n`,
      "utf8",
    );

    assert.throws(
      () => validateActivationGate({ root: fixture.root }),
      /detached signature is not valid/u,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("signed manifest rejects chain-spec content changed after approval", () => {
  const fixture = writeSignedFixture();
  try {
    const specPath = join(fixture.root, "testnet", "activation.json");
    const spec = JSON.parse(readFileSync(specPath, "utf8"));
    spec.services.metrics = "https://tampered.invalid";
    writeJson(specPath, spec);

    assert.throws(
      () => validateActivationGate({ root: fixture.root }),
      /canonical spec digest mismatch/u,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("signed manifest rejects a missing required ecosystem authority", () => {
  const fixture = writeSignedFixture({
    policies: [policy("npm_trusted_publisher", "a")],
  });
  try {
    assert.throws(
      () => validateActivationGate({ root: fixture.root }),
      /missing required authority pypi_trusted_publisher/u,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("signed manifest rejects a conflicting distribution mirror", () => {
  const mirror = {
    spec_path: "testnet/mirror/activation.json",
    signature_path: "testnet/mirror/activation.json.sig",
  };
  const fixture = writeSignedFixture({ mirrors: [mirror] });
  try {
    const mirrorPath = join(fixture.root, mirror.spec_path);
    const conflicting = JSON.parse(readFileSync(mirrorPath, "utf8"));
    conflicting.services.metrics = "https://conflicting.invalid";
    writeJson(mirrorPath, conflicting);

    assert.throws(
      () => validateActivationGate({ root: fixture.root }),
      /is not canonically identical to the selected spec/u,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("signed manifest rejects placeholder policy and trust-root pins", () => {
  const fixture = writeSignedFixture();
  try {
    const specPath = join(fixture.root, "testnet", "activation.json");
    const spec = JSON.parse(readFileSync(specPath, "utf8"));
    spec.consensus_params.namespace_authority_policies[0].trust_root_sha256 =
      "0".repeat(64);
    writeJson(specPath, spec);

    assert.throws(
      () => validateActivationGate({ root: fixture.root }),
      /must not be an all-zero placeholder digest/u,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("signed manifest rejects an explicitly disallowed signing key", () => {
  const fixture = writeSignedFixture();
  try {
    fixture.manifest.disallowed_signing_pubkeys = [
      fixture.spec.signing.signing_key_pubkey_hex,
    ];
    writeJson(
      join(fixture.root, "config", "cr07-activation-manifest.json"),
      fixture.manifest,
    );

    assert.throws(
      () => validateActivationGate({ root: fixture.root }),
      /is explicitly disallowed and must be rotated/u,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("unknown activation manifest versions fail closed", () => {
  const fixture = writeSignedFixture();
  try {
    fixture.manifest.manifest_version = 2;
    writeJson(
      join(fixture.root, "config", "cr07-activation-manifest.json"),
      fixture.manifest,
    );

    assert.throws(
      () => validateActivationGate({ root: fixture.root }),
      /unknown versions fail closed/u,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});
