#!/usr/bin/env node

import {
  createHash,
  createPublicKey,
  verify as verifySignature,
} from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const ACTIVATION_DOMAIN = "creg-cr07-activation-v1";
export const CHAIN_SPEC_SIGNATURE_DOMAIN = "creg-chain-spec-v1";
export const CURRENT_MANIFEST_VERSION = 1;
export const CURRENT_CHAIN_SPEC_VERSION = 1;

const SHA256_RE = /^[0-9a-f]{64}$/;
const GENESIS_HASH_RE = /^0x[0-9a-f]{64}$/;
const ED25519_PUBLIC_KEY_RE = /^(?:0x)?[0-9a-f]{64}$/;
const ED25519_SIGNATURE_RE = /^(?:0x)?[0-9a-f]{128}$/;
const AUTHORITIES = new Set([
  "npm_trusted_publisher",
  "pypi_trusted_publisher",
]);

function fail(message) {
  throw new Error(`CR-07 activation gate: ${message}`);
}

function requireObject(value, path) {
  if (
    value === null ||
    typeof value !== "object" ||
    Array.isArray(value)
  ) {
    fail(`${path} must be an object`);
  }
  return value;
}

function requireExactKeys(value, allowed, path) {
  const unexpected = Object.keys(value).filter((key) => !allowed.has(key));
  if (unexpected.length > 0) {
    fail(`${path} contains unknown field(s): ${unexpected.join(", ")}`);
  }
}

function requireNonEmptyString(value, path) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.trim() !== value ||
    /[\r\n\0]/u.test(value)
  ) {
    fail(`${path} must be a canonical non-empty single-line string`);
  }
  return value;
}

function requireLowerSha256(value, path, { allowZero = false } = {}) {
  if (typeof value !== "string" || !SHA256_RE.test(value)) {
    fail(`${path} must be a lowercase SHA-256 hex digest`);
  }
  if (!allowZero && /^0{64}$/.test(value)) {
    fail(`${path} must not be an all-zero placeholder digest`);
  }
  return value;
}

function requireSafeInteger(value, path) {
  if (!Number.isSafeInteger(value) || value < 0) {
    fail(`${path} must be a non-negative safe integer`);
  }
  return value;
}

function assertJsonNumbersAreSafe(value, path = "$") {
  if (typeof value === "number") {
    if (!Number.isFinite(value) || !Number.isSafeInteger(value)) {
      fail(`${path} contains a non-canonical or unsafe JSON number`);
    }
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((item, index) =>
      assertJsonNumbersAreSafe(item, `${path}[${index}]`),
    );
    return;
  }
  if (value && typeof value === "object") {
    for (const [key, item] of Object.entries(value)) {
      assertJsonNumbersAreSafe(item, `${path}.${key}`);
    }
  }
}

/**
 * RFC 8785-compatible serialization for the data model accepted by this gate.
 * Chain specs represent integers larger than JavaScript's safe range as
 * strings; unsafe JSON numbers are rejected before this function is called.
 */
export function canonicalJson(value) {
  assertJsonNumbersAreSafe(value);

  if (value === null || typeof value !== "object") {
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map((item) => canonicalJson(item)).join(",")}]`;
  }

  return `{${Object.keys(value)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
    .join(",")}}`;
}

export function sha256Hex(value) {
  return createHash("sha256").update(value).digest("hex");
}

function readJson(path, label) {
  let raw;
  try {
    raw = readFileSync(path, "utf8");
  } catch (error) {
    fail(`cannot read ${label} at ${path}: ${error.message}`);
  }

  try {
    const parsed = JSON.parse(raw);
    assertJsonNumbersAreSafe(parsed, label);
    return parsed;
  } catch (error) {
    if (error.message.startsWith("CR-07 activation gate:")) {
      throw error;
    }
    fail(`${label} at ${path} is not valid JSON: ${error.message}`);
  }
}

function resolveRepositoryPath(root, value, path) {
  requireNonEmptyString(value, path);
  if (isAbsolute(value)) {
    fail(`${path} must be repository-relative`);
  }

  const absolute = resolve(root, value);
  const fromRoot = relative(root, absolute);
  if (fromRoot === ".." || fromRoot.startsWith(`..\\`) || fromRoot.startsWith("../")) {
    fail(`${path} escapes the repository root`);
  }
  return absolute;
}

function validatePolicy(policy, path) {
  requireObject(policy, path);
  requireExactKeys(
    policy,
    new Set([
      "authority",
      "adapter_id",
      "adapter_version",
      "verifier_sha256",
      "policy_sha256",
      "trust_root_sha256",
    ]),
    path,
  );

  if (!AUTHORITIES.has(policy.authority)) {
    fail(`${path}.authority is not a supported CR-07 authority`);
  }
  requireNonEmptyString(policy.adapter_id, `${path}.adapter_id`);
  requireNonEmptyString(policy.adapter_version, `${path}.adapter_version`);
  requireLowerSha256(policy.verifier_sha256, `${path}.verifier_sha256`);
  requireLowerSha256(policy.policy_sha256, `${path}.policy_sha256`);
  requireLowerSha256(policy.trust_root_sha256, `${path}.trust_root_sha256`);
}

function cr07State(spec, path) {
  const consensus = requireObject(spec.consensus_params, `${path}.consensus_params`);
  const hasHeight =
    Object.hasOwn(consensus, "namespace_authority_activation_height") &&
    consensus.namespace_authority_activation_height !== null;
  const policies = consensus.namespace_authority_policies ?? [];

  if (!Array.isArray(policies)) {
    fail(`${path}.consensus_params.namespace_authority_policies must be an array`);
  }

  if (hasHeight) {
    requireSafeInteger(
      consensus.namespace_authority_activation_height,
      `${path}.consensus_params.namespace_authority_activation_height`,
    );
  }

  if (hasHeight !== (policies.length > 0)) {
    fail(
      `${path} must set activation height and non-empty policy pins together`,
    );
  }

  policies.forEach((policy, index) =>
    validatePolicy(
      policy,
      `${path}.consensus_params.namespace_authority_policies[${index}]`,
    ),
  );

  const identities = new Set();
  for (const policy of policies) {
    const identity = [
      policy.authority,
      policy.adapter_id,
      policy.adapter_version,
    ].join("|");
    if (identities.has(identity)) {
      fail(`${path} contains a duplicate authority adapter identity`);
    }
    identities.add(identity);
  }

  return {
    active: hasHeight,
    height: hasHeight
      ? consensus.namespace_authority_activation_height
      : null,
    policies,
  };
}

function validateChainSpecEnvelope(spec, path, expectedVersion) {
  requireObject(spec, path);
  if (spec.spec_version !== expectedVersion) {
    fail(
      `${path}.spec_version must be ${expectedVersion}, got ${String(spec.spec_version)}`,
    );
  }
  requireNonEmptyString(spec.chain_id, `${path}.chain_id`);
  requireObject(spec.signing, `${path}.signing`);
  if (spec.signing.signature_algorithm !== "ed25519") {
    fail(`${path}.signing.signature_algorithm must be ed25519`);
  }
  if (
    typeof spec.signing.signing_key_pubkey_hex !== "string" ||
    !ED25519_PUBLIC_KEY_RE.test(spec.signing.signing_key_pubkey_hex)
  ) {
    fail(`${path}.signing.signing_key_pubkey_hex must be a lowercase Ed25519 key`);
  }
  requireNonEmptyString(
    spec.signing.detached_signature_url,
    `${path}.signing.detached_signature_url`,
  );

  return cr07State(spec, path);
}

export function computeGenesisHash(spec) {
  const validatorSet = requireObject(spec.validator_set, "chain spec.validator_set");
  if (!Array.isArray(validatorSet.validators)) {
    fail("chain spec.validator_set.validators must be an array");
  }
  const l1 = requireObject(spec.l1, "chain spec.l1");

  const payload = {
    chain_id: spec.chain_id,
    genesis_time: spec.genesis_time,
    consensus_params: spec.consensus_params,
    feature_flags: spec.feature_flags,
    l1: { chain_id: l1.chain_id },
    contracts: spec.contracts,
    validator_set: {
      version: validatorSet.version,
      validators: validatorSet.validators.map((validator) => [
        validator.id,
        validator.pubkey,
        validator.eth_address,
        validator.stake,
      ]),
    },
  };

  return `0x${sha256Hex(canonicalJson(payload))}`;
}

export function activationPayloadDigest(spec, state) {
  return sha256Hex(
    canonicalJson({
      domain: ACTIVATION_DOMAIN,
      chain_id: spec.chain_id,
      genesis_hash: spec.genesis_hash,
      activation_height: state.height,
      policies: state.policies,
    }),
  );
}

export function inspectActivationCandidate({
  root = process.cwd(),
  specPath,
} = {}) {
  root = resolve(root);
  const absolute = resolveRepositoryPath(root, specPath, "candidate spec path");
  const spec = readJson(absolute, specPath);
  const state = validateChainSpecEnvelope(
    spec,
    specPath,
    CURRENT_CHAIN_SPEC_VERSION,
  );
  if (!state.active) {
    fail(`${specPath} does not contain a CR-07 activation`);
  }
  const computedGenesis = computeGenesisHash(spec);
  if (computedGenesis !== spec.genesis_hash) {
    fail(
      `candidate chain spec genesis hash mismatch: declared=${spec.genesis_hash} computed=${computedGenesis}`,
    );
  }
  return {
    chain_id: spec.chain_id,
    genesis_hash: computedGenesis,
    signing_pubkey_hex: spec.signing.signing_key_pubkey_hex,
    activation_height: state.height,
    activation_payload_sha256: activationPayloadDigest(spec, state),
    canonical_spec_sha256: sha256Hex(canonicalJson(spec)),
  };
}

function readSignature(path, label) {
  let signature;
  try {
    signature = readFileSync(path, "utf8").trim();
  } catch (error) {
    fail(`cannot read ${label} at ${path}: ${error.message}`);
  }
  if (!ED25519_SIGNATURE_RE.test(signature)) {
    fail(`${label} must contain exactly one hex-encoded Ed25519 signature`);
  }
  return signature.replace(/^0x/u, "");
}

export function verifyChainSpecSignature(spec, signature, path) {
  const publicKeyHex = spec.signing.signing_key_pubkey_hex.replace(/^0x/u, "");
  const spkiPrefix = Buffer.from("302a300506032b6570032100", "hex");
  const publicKey = createPublicKey({
    key: Buffer.concat([spkiPrefix, Buffer.from(publicKeyHex, "hex")]),
    format: "der",
    type: "spki",
  });
  const message = Buffer.from(
    `${CHAIN_SPEC_SIGNATURE_DOMAIN}|${canonicalJson(spec)}`,
    "utf8",
  );
  if (
    !verifySignature(
      null,
      message,
      publicKey,
      Buffer.from(signature, "hex"),
    )
  ) {
    fail(`${path} detached signature is not valid for the embedded signing key`);
  }
}

function validateRequiredAuthorities(required) {
  if (!Array.isArray(required) || required.length === 0) {
    fail("manifest.required_authorities must be a non-empty array");
  }
  const unique = new Set();
  for (const [index, authority] of required.entries()) {
    if (!AUTHORITIES.has(authority)) {
      fail(
        `manifest.required_authorities[${index}] is not a supported authority`,
      );
    }
    if (unique.has(authority)) {
      fail(`manifest.required_authorities contains duplicate ${authority}`);
    }
    unique.add(authority);
  }
  return unique;
}

function validateDisallowedSigningKeys(keys) {
  if (!Array.isArray(keys)) {
    fail("manifest.disallowed_signing_pubkeys must be an array");
  }
  const normalized = new Set();
  for (const [index, key] of keys.entries()) {
    if (
      typeof key !== "string" ||
      !ED25519_PUBLIC_KEY_RE.test(key) ||
      /^(?:0x)?0{64}$/u.test(key)
    ) {
      fail(
        `manifest.disallowed_signing_pubkeys[${index}] must be a non-zero lowercase Ed25519 key`,
      );
    }
    const value = key.replace(/^0x/u, "");
    if (normalized.has(value)) {
      fail(`manifest.disallowed_signing_pubkeys contains duplicate ${key}`);
    }
    normalized.add(value);
  }
  return normalized;
}

function validateActivationDescriptor(activation) {
  requireObject(activation, "manifest.activation");
  requireExactKeys(
    activation,
    new Set([
      "spec_path",
      "signature_path",
      "mirrors",
      "chain_id",
      "genesis_hash",
      "signing_pubkey_hex",
      "activation_height",
      "activation_payload_sha256",
      "canonical_spec_sha256",
    ]),
    "manifest.activation",
  );
  requireNonEmptyString(activation.spec_path, "manifest.activation.spec_path");
  requireNonEmptyString(
    activation.signature_path,
    "manifest.activation.signature_path",
  );
  requireNonEmptyString(activation.chain_id, "manifest.activation.chain_id");
  if (
    typeof activation.genesis_hash !== "string" ||
    !GENESIS_HASH_RE.test(activation.genesis_hash) ||
    /^0x0{64}$/u.test(activation.genesis_hash)
  ) {
    fail(
      "manifest.activation.genesis_hash must be a non-zero lowercase 0x-prefixed digest",
    );
  }
  if (
    typeof activation.signing_pubkey_hex !== "string" ||
    !ED25519_PUBLIC_KEY_RE.test(activation.signing_pubkey_hex) ||
    /^(?:0x)?0{64}$/u.test(activation.signing_pubkey_hex)
  ) {
    fail("manifest.activation.signing_pubkey_hex must be a non-zero Ed25519 key");
  }
  requireSafeInteger(
    activation.activation_height,
    "manifest.activation.activation_height",
  );
  if (activation.activation_height === 0) {
    fail(
      "manifest.activation.activation_height must be greater than zero for a staged rollout",
    );
  }
  requireLowerSha256(
    activation.activation_payload_sha256,
    "manifest.activation.activation_payload_sha256",
  );
  requireLowerSha256(
    activation.canonical_spec_sha256,
    "manifest.activation.canonical_spec_sha256",
  );

  if (!Array.isArray(activation.mirrors)) {
    fail("manifest.activation.mirrors must be an array");
  }
  for (const [index, mirror] of activation.mirrors.entries()) {
    requireObject(mirror, `manifest.activation.mirrors[${index}]`);
    requireExactKeys(
      mirror,
      new Set(["spec_path", "signature_path"]),
      `manifest.activation.mirrors[${index}]`,
    );
    requireNonEmptyString(
      mirror.spec_path,
      `manifest.activation.mirrors[${index}].spec_path`,
    );
    requireNonEmptyString(
      mirror.signature_path,
      `manifest.activation.mirrors[${index}].signature_path`,
    );
  }
}

export function validateActivationGate({
  root = process.cwd(),
  manifestPath = "config/cr07-activation-manifest.json",
} = {}) {
  root = resolve(root);
  const absoluteManifestPath = resolveRepositoryPath(
    root,
    manifestPath,
    "manifest path",
  );
  const manifest = readJson(absoluteManifestPath, "manifest");
  requireObject(manifest, "manifest");
  requireExactKeys(
    manifest,
    new Set([
      "manifest_version",
      "status",
      "signature_domain",
      "chain_spec_version",
      "required_authorities",
      "disallowed_signing_pubkeys",
      "guarded_chain_specs",
      "activation",
    ]),
    "manifest",
  );

  if (manifest.manifest_version !== CURRENT_MANIFEST_VERSION) {
    fail(
      `manifest.manifest_version must be ${CURRENT_MANIFEST_VERSION}; unknown versions fail closed`,
    );
  }
  if (!["disabled", "signed"].includes(manifest.status)) {
    fail("manifest.status must be disabled or signed");
  }
  if (manifest.signature_domain !== CHAIN_SPEC_SIGNATURE_DOMAIN) {
    fail(
      `manifest.signature_domain must be ${CHAIN_SPEC_SIGNATURE_DOMAIN}`,
    );
  }
  if (manifest.chain_spec_version !== CURRENT_CHAIN_SPEC_VERSION) {
    fail(
      `manifest.chain_spec_version must be ${CURRENT_CHAIN_SPEC_VERSION}`,
    );
  }

  const requiredAuthorities = validateRequiredAuthorities(
    manifest.required_authorities,
  );
  const disallowedSigningKeys = validateDisallowedSigningKeys(
    manifest.disallowed_signing_pubkeys,
  );
  if (
    !Array.isArray(manifest.guarded_chain_specs) ||
    manifest.guarded_chain_specs.length === 0
  ) {
    fail("manifest.guarded_chain_specs must be a non-empty array");
  }

  const guarded = new Map();
  for (const [index, repositoryPath] of manifest.guarded_chain_specs.entries()) {
    const label = `manifest.guarded_chain_specs[${index}]`;
    const absolute = resolveRepositoryPath(root, repositoryPath, label);
    if (guarded.has(repositoryPath)) {
      fail(`manifest.guarded_chain_specs contains duplicate ${repositoryPath}`);
    }
    const spec = readJson(absolute, repositoryPath);
    const state = validateChainSpecEnvelope(
      spec,
      repositoryPath,
      manifest.chain_spec_version,
    );
    guarded.set(repositoryPath, { absolute, spec, state });
  }

  if (manifest.status === "disabled") {
    if (manifest.activation !== null) {
      fail("manifest.activation must be null while status is disabled");
    }
    for (const [path, record] of guarded) {
      if (record.state.active) {
        fail(`${path} activates CR-07 while the repository manifest is disabled`);
      }
    }
    return {
      status: manifest.status,
      guardedSpecs: guarded.size,
      activationSpec: null,
    };
  }

  validateActivationDescriptor(manifest.activation);
  const activation = manifest.activation;
  const selectedPaths = [
    activation.spec_path,
    ...activation.mirrors.map((mirror) => mirror.spec_path),
  ];
  if (new Set(selectedPaths).size !== selectedPaths.length) {
    fail("manifest.activation spec and mirror paths must be unique");
  }
  for (const path of selectedPaths) {
    if (!guarded.has(path)) {
      fail(`${path} must also appear in manifest.guarded_chain_specs`);
    }
  }

  for (const [path, record] of guarded) {
    if (selectedPaths.includes(path)) {
      if (!record.state.active) {
        fail(`${path} is selected for activation but has no CR-07 activation`);
      }
    } else if (record.state.active) {
      fail(`${path} contains an unmanifested CR-07 activation`);
    }
  }

  const primary = guarded.get(activation.spec_path);
  const spec = primary.spec;
  const state = primary.state;
  if (spec.chain_id !== activation.chain_id) {
    fail("activation chain_id does not match the selected chain spec");
  }
  if (spec.genesis_hash !== activation.genesis_hash) {
    fail("activation genesis_hash does not match the selected chain spec");
  }
  if (
    spec.signing.signing_key_pubkey_hex.replace(/^0x/u, "") !==
    activation.signing_pubkey_hex.replace(/^0x/u, "")
  ) {
    fail("activation signing_pubkey_hex does not match the selected chain spec");
  }
  if (
    disallowedSigningKeys.has(
      activation.signing_pubkey_hex.replace(/^0x/u, ""),
    )
  ) {
    fail(
      "activation signing_pubkey_hex is explicitly disallowed and must be rotated before CR-07 activation",
    );
  }
  if (state.height !== activation.activation_height) {
    fail("activation height does not match the selected chain spec");
  }

  const computedGenesis = computeGenesisHash(spec);
  if (computedGenesis !== spec.genesis_hash) {
    fail(
      `selected chain spec genesis hash mismatch: declared=${spec.genesis_hash} computed=${computedGenesis}`,
    );
  }
  const computedSpecDigest = sha256Hex(canonicalJson(spec));
  if (computedSpecDigest !== activation.canonical_spec_sha256) {
    fail(
      `canonical spec digest mismatch: declared=${activation.canonical_spec_sha256} computed=${computedSpecDigest}`,
    );
  }
  const computedActivationDigest = activationPayloadDigest(spec, state);
  if (computedActivationDigest !== activation.activation_payload_sha256) {
    fail(
      `activation payload digest mismatch: declared=${activation.activation_payload_sha256} computed=${computedActivationDigest}`,
    );
  }

  const presentAuthorities = new Set(
    state.policies.map((policy) => policy.authority),
  );
  for (const authority of requiredAuthorities) {
    if (!presentAuthorities.has(authority)) {
      fail(`selected chain spec is missing required authority ${authority}`);
    }
  }

  const primarySignaturePath = resolveRepositoryPath(
    root,
    activation.signature_path,
    "manifest.activation.signature_path",
  );
  const primarySignature = readSignature(
    primarySignaturePath,
    "activation signature",
  );
  verifyChainSpecSignature(spec, primarySignature, activation.spec_path);

  const primaryCanonical = canonicalJson(spec);
  for (const [index, mirror] of activation.mirrors.entries()) {
    const mirrorRecord = guarded.get(mirror.spec_path);
    if (canonicalJson(mirrorRecord.spec) !== primaryCanonical) {
      fail(
        `manifest.activation.mirrors[${index}] is not canonically identical to the selected spec`,
      );
    }
    const signaturePath = resolveRepositoryPath(
      root,
      mirror.signature_path,
      `manifest.activation.mirrors[${index}].signature_path`,
    );
    const signature = readSignature(
      signaturePath,
      `manifest.activation.mirrors[${index}] signature`,
    );
    if (signature !== primarySignature) {
      fail(
        `manifest.activation.mirrors[${index}] detached signature differs from the selected signature`,
      );
    }
    verifyChainSpecSignature(mirrorRecord.spec, signature, mirror.spec_path);
  }

  return {
    status: manifest.status,
    guardedSpecs: guarded.size,
    activationSpec: activation.spec_path,
    genesisHash: computedGenesis,
    canonicalSpecSha256: computedSpecDigest,
    activationPayloadSha256: computedActivationDigest,
  };
}

function usage() {
  console.log(`Usage: node scripts/validate-cr07-activation.mjs [--manifest PATH] [--root PATH]
       node scripts/validate-cr07-activation.mjs --inspect-spec PATH [--root PATH]

Validates the repository CR-07 activation manifest. A disabled manifest rejects
CR-07 fields in every guarded chain spec. A signed manifest additionally
requires exact capability hashes, genesis identity, mirrors, and a valid
detached Ed25519 chain-spec signature.

--inspect-spec prints the candidate values needed in manifest.activation. It
does not verify a detached signature or authorize activation.`);
}

async function main() {
  let root = process.cwd();
  let manifestPath = "config/cr07-activation-manifest.json";
  let inspectSpecPath = null;

  for (let index = 2; index < process.argv.length; index += 1) {
    const argument = process.argv[index];
    if (argument === "--root") {
      root = process.argv[++index];
      if (!root) fail("--root requires a value");
    } else if (argument === "--manifest") {
      manifestPath = process.argv[++index];
      if (!manifestPath) fail("--manifest requires a value");
    } else if (argument === "--inspect-spec") {
      inspectSpecPath = process.argv[++index];
      if (!inspectSpecPath) fail("--inspect-spec requires a value");
    } else if (argument === "--help" || argument === "-h") {
      usage();
      return;
    } else {
      fail(`unknown argument: ${argument}`);
    }
  }

  if (inspectSpecPath !== null) {
    console.error(
      "Candidate digests only; detached signature and manifest authorization have not been verified.",
    );
    console.log(
      JSON.stringify(
        inspectActivationCandidate({ root, specPath: inspectSpecPath }),
        null,
        2,
      ),
    );
    return;
  }

  const result = validateActivationGate({ root, manifestPath });
  if (result.status === "disabled") {
    console.log(
      `CR-07 activation is disabled; ${result.guardedSpecs} guarded chain specs contain no activation fields`,
    );
  } else {
    console.log(
      `CR-07 signed activation is valid: ${result.activationSpec} (${result.activationPayloadSha256})`,
    );
  }
}

const invokedPath = process.argv[1]
  ? resolve(process.argv[1])
  : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
