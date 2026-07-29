#!/usr/bin/env node

import {
  createHash,
  createPrivateKey,
  createPublicKey,
  sign as signMessage,
  verify as verifyMessage,
} from "node:crypto";
import {
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const CHECKPOINT_EVIDENCE_DOMAIN =
  "creg-weak-subjectivity-checkpoint-evidence-v1";
export const CHECKPOINT_EVIDENCE_KIND =
  "creg-weak-subjectivity-checkpoint-verification";
export const CHECKPOINT_EVIDENCE_SCHEMA_VERSION = 1;
export const VALIDATOR_SNAPSHOT_SCHEMA_VERSION = 1;
export const MAX_VALIDATOR_SET_MEMBERS = 10_000;
export const MAX_CHECKPOINT_ENVELOPE_BYTES = 1024 * 1024;

const TOOL_VERSION = "1.0.0";
const MAX_INPUT_FILE_BYTES = 16 * 1024 * 1024;
const MAX_PRIVATE_KEY_BYTES = 64 * 1024;
const LOWER_HASH_RE = /^[0-9a-f]{64}$/u;
const LOWER_SIGNATURE_RE = /^[0-9a-f]{128}$/u;
const UTC_SECONDS_RE = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/u;
const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

function fail(message) {
  throw new Error(`weak-subjectivity checkpoint gate: ${message}`);
}

function requireObject(value, path) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${path} must be an object`);
  }
  return value;
}

function requireExactKeys(value, required, optional, path) {
  const allowed = new Set([...required, ...optional]);
  const unknown = Object.keys(value).filter((key) => !allowed.has(key));
  if (unknown.length > 0) {
    fail(`${path} contains unknown field(s): ${unknown.join(", ")}`);
  }
  const missing = required.filter(
    (key) => !Object.prototype.hasOwnProperty.call(value, key),
  );
  if (missing.length > 0) {
    fail(`${path} is missing field(s): ${missing.join(", ")}`);
  }
}

function requireString(value, path, { allowEmpty = false } = {}) {
  if (
    typeof value !== "string" ||
    (!allowEmpty && value.length === 0) ||
    value.trim() !== value ||
    /[\r\n\0]/u.test(value)
  ) {
    fail(`${path} must be a canonical single-line string`);
  }
  return value;
}

function requireLowerHash(value, path) {
  if (typeof value !== "string" || !LOWER_HASH_RE.test(value)) {
    fail(`${path} must be a lowercase 32-byte hex digest`);
  }
  return value;
}

function requireNonZeroHash(value, path) {
  requireLowerHash(value, path);
  if (/^0{64}$/u.test(value)) {
    fail(`${path} must not be an all-zero placeholder digest`);
  }
  return value;
}

function requireLowerSignature(value, path) {
  if (typeof value !== "string" || !LOWER_SIGNATURE_RE.test(value)) {
    fail(`${path} must be a lowercase 64-byte Ed25519 signature`);
  }
  return value;
}

function requireSafeInteger(value, path, maximum = Number.MAX_SAFE_INTEGER) {
  if (!Number.isSafeInteger(value) || value < 0 || value > maximum) {
    fail(`${path} must be a non-negative safe integer no greater than ${maximum}`);
  }
  return value;
}

function requirePositiveSafeInteger(value, path) {
  requireSafeInteger(value, path);
  if (value === 0) {
    fail(`${path} must be greater than zero`);
  }
  return value;
}

function requireUtcSeconds(value, path) {
  if (
    typeof value !== "string" ||
    !UTC_SECONDS_RE.test(value) ||
    Number.isNaN(Date.parse(value))
  ) {
    fail(`${path} must be an RFC 3339 UTC timestamp with whole seconds`);
  }
  return value;
}

function compareRustStrings(left, right) {
  return Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8"));
}

function assertJsonNumbersAreSafe(value, path = "$") {
  if (typeof value === "number") {
    if (!Number.isSafeInteger(value)) {
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
 * RFC 8785-compatible for the integer/string/object data model used here.
 * Unsafe JSON numbers are rejected before serialization.
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

function normalizeValidator(value, path) {
  requireObject(value, path);
  requireExactKeys(
    value,
    ["id", "alias", "pubkey", "stake", "reputation", "status"],
    ["eth_address", "performance_score", "contribution_count"],
    path,
  );
  const pubkey = requireLowerHash(value.pubkey, `${path}.pubkey`);
  return {
    // Keep this field order synchronized with common::Validator. Rust's
    // serde_json serializer uses declaration order for the consensus hash.
    id: requireString(value.id, `${path}.id`),
    alias: requireString(value.alias, `${path}.alias`, { allowEmpty: true }),
    pubkey,
    eth_address: requireString(
      value.eth_address ?? "",
      `${path}.eth_address`,
      { allowEmpty: true },
    ),
    stake: requireSafeInteger(value.stake, `${path}.stake`),
    reputation: requireSafeInteger(
      value.reputation,
      `${path}.reputation`,
      0xffff_ffff,
    ),
    status: requireString(value.status, `${path}.status`, { allowEmpty: true }),
    performance_score: requireSafeInteger(
      value.performance_score ?? 500,
      `${path}.performance_score`,
      0xffff_ffff,
    ),
    contribution_count: requireSafeInteger(
      value.contribution_count ?? 0,
      `${path}.contribution_count`,
      0xffff_ffff,
    ),
  };
}

export function normalizeValidatorSet(value, path = "validator_set") {
  requireObject(value, path);
  requireExactKeys(value, ["validators"], [], path);
  if (
    !Array.isArray(value.validators) ||
    value.validators.length === 0 ||
    value.validators.length > MAX_VALIDATOR_SET_MEMBERS
  ) {
    fail(
      `${path}.validators must contain 1..${MAX_VALIDATOR_SET_MEMBERS} members`,
    );
  }
  const validators = value.validators.map((validator, index) =>
    normalizeValidator(validator, `${path}.validators[${index}]`),
  );
  const ids = new Set();
  const pubkeys = new Set();
  for (const validator of validators) {
    if (ids.has(validator.id)) {
      fail(`${path} contains duplicate validator identity ${validator.id}`);
    }
    if (pubkeys.has(validator.pubkey)) {
      fail(`${path} contains duplicate validator public key`);
    }
    ids.add(validator.id);
    pubkeys.add(validator.pubkey);
  }
  return { validators };
}

function normalizeTransitionSignature(value, path) {
  requireObject(value, path);
  requireExactKeys(
    value,
    ["validator_id", "validator_pubkey", "signature"],
    [],
    path,
  );
  return {
    validator_id: requireString(value.validator_id, `${path}.validator_id`),
    validator_pubkey: requireLowerHash(
      value.validator_pubkey,
      `${path}.validator_pubkey`,
    ),
    signature: requireLowerSignature(value.signature, `${path}.signature`),
  };
}

export function normalizeCheckpoint(value, path = "checkpoint") {
  requireObject(value, path);
  requireExactKeys(
    value,
    [
      "schema_version",
      "effective_from_height",
      "previous_block_hash",
      "previous_validator_set_hash",
      "validators",
      "validator_set_hash",
      "transition_signatures",
    ],
    [],
    path,
  );
  if (value.schema_version !== VALIDATOR_SNAPSHOT_SCHEMA_VERSION) {
    fail(`${path}.schema_version is unsupported`);
  }
  if (
    !Array.isArray(value.transition_signatures) ||
    value.transition_signatures.length === 0 ||
    value.transition_signatures.length > MAX_VALIDATOR_SET_MEMBERS
  ) {
    fail(
      `${path}.transition_signatures must contain 1..${MAX_VALIDATOR_SET_MEMBERS} witnesses`,
    );
  }
  const transitionSignatures = value.transition_signatures.map(
    (signature, index) =>
      normalizeTransitionSignature(
        signature,
        `${path}.transition_signatures[${index}]`,
      ),
  );
  const signerIds = new Set();
  for (const signature of transitionSignatures) {
    if (signerIds.has(signature.validator_id)) {
      fail(`${path} contains duplicate signer identity ${signature.validator_id}`);
    }
    signerIds.add(signature.validator_id);
  }

  const checkpoint = {
    // Keep this field order synchronized with
    // common::AuthenticatedValidatorSetSnapshot.
    schema_version: value.schema_version,
    effective_from_height: requirePositiveSafeInteger(
      value.effective_from_height,
      `${path}.effective_from_height`,
    ),
    previous_block_hash: requireNonZeroHash(
      value.previous_block_hash,
      `${path}.previous_block_hash`,
    ),
    previous_validator_set_hash: requireLowerHash(
      value.previous_validator_set_hash,
      `${path}.previous_validator_set_hash`,
    ),
    validators: normalizeValidatorSet(value.validators, `${path}.validators`),
    validator_set_hash: requireLowerHash(
      value.validator_set_hash,
      `${path}.validator_set_hash`,
    ),
    transition_signatures: transitionSignatures,
  };
  const computedSetHash = validatorSetHash(checkpoint.validators);
  if (checkpoint.validator_set_hash !== computedSetHash) {
    fail(
      `${path}.validator_set_hash mismatch: declared=${checkpoint.validator_set_hash} computed=${computedSetHash}`,
    );
  }
  return checkpoint;
}

export function validatorSetHash(value) {
  const set = normalizeValidatorSet(value);
  const validators = [...set.validators].sort(
    (left, right) =>
      compareRustStrings(left.id, right.id) ||
      compareRustStrings(left.pubkey.toLowerCase(), right.pubkey.toLowerCase()),
  );
  // Rust hashes serde_json::to_vec(&validators), not the ValidatorSet wrapper.
  return sha256Hex(JSON.stringify(validators));
}

export function transitionMessage(checkpointInput) {
  const checkpoint = normalizeCheckpoint(checkpointInput);
  return [
    "creg-validator-set-transition-v1",
    "service:creg-consensus",
    `effective_from_height:${checkpoint.effective_from_height}`,
    `previous_block_hash:${checkpoint.previous_block_hash}`,
    `previous_validator_set_hash:${checkpoint.previous_validator_set_hash}`,
    `validator_set_hash:${checkpoint.validator_set_hash}`,
  ].join("\n");
}

export function logicalTransitionId(checkpointInput) {
  const checkpoint = normalizeCheckpoint(checkpointInput);
  const message = transitionMessage(checkpoint);
  return sha256Hex(
    [
      "creg-certified-validator-set-transition-id-v1",
      `schema_version:${checkpoint.schema_version}`,
      message,
    ].join("\n"),
  );
}

export function witnessHash(checkpointInput) {
  const checkpoint = normalizeCheckpoint(checkpointInput);
  const sorted = {
    ...checkpoint,
    transition_signatures: [...checkpoint.transition_signatures].sort(
      (left, right) =>
        compareRustStrings(left.validator_id, right.validator_id) ||
        compareRustStrings(left.validator_pubkey, right.validator_pubkey) ||
        compareRustStrings(left.signature, right.signature),
    ),
  };
  return sha256Hex(JSON.stringify(sorted));
}

function publicKeyFromRawHex(value, path) {
  requireLowerHash(value, path);
  return createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, Buffer.from(value, "hex")]),
    format: "der",
    type: "spki",
  });
}

export function validateCheckpointQuorum(checkpointInput, previousSetInput) {
  const checkpoint = normalizeCheckpoint(checkpointInput);
  const previousSet = normalizeValidatorSet(
    previousSetInput,
    "previous_validator_set",
  );
  const computedPreviousHash = validatorSetHash(previousSet);
  if (checkpoint.previous_validator_set_hash !== computedPreviousHash) {
    fail(
      `checkpoint previous-validator-set hash mismatch: declared=${checkpoint.previous_validator_set_hash} computed=${computedPreviousHash}`,
    );
  }
  if (checkpoint.transition_signatures.length > previousSet.validators.length) {
    fail("checkpoint witness exceeds the prior validator-set size");
  }

  const validatorsById = new Map(
    previousSet.validators.map((validator) => [validator.id, validator]),
  );
  const message = Buffer.from(transitionMessage(checkpoint), "utf8");
  const validSigners = [];
  for (const [index, witness] of checkpoint.transition_signatures.entries()) {
    const validator = validatorsById.get(witness.validator_id);
    if (!validator) {
      fail(`checkpoint witness ${index} names an unknown prior-set signer`);
    }
    if (validator.pubkey !== witness.validator_pubkey) {
      fail(`checkpoint witness ${index} public key does not match its signer`);
    }
    const valid = verifyMessage(
      null,
      message,
      publicKeyFromRawHex(validator.pubkey, `prior signer ${validator.id}`),
      Buffer.from(witness.signature, "hex"),
    );
    if (!valid) {
      fail(`checkpoint witness ${index} has an invalid Ed25519 signature`);
    }
    validSigners.push(validator.id);
  }
  const quorumRequired = Math.floor(
    (2 * previousSet.validators.length) / 3,
  ) + 1;
  if (validSigners.length < quorumRequired) {
    fail(
      `checkpoint lacks prior-set quorum: valid=${validSigners.length} required=${quorumRequired}`,
    );
  }
  return {
    checkpoint,
    previousSet,
    validSigners: [...validSigners].sort(compareRustStrings),
    quorumRequired,
  };
}

export function normalizeCanonicalAnchor(value, path = "canonical_anchor") {
  requireObject(value, path);
  requireExactKeys(
    value,
    [
      "schema_version",
      "chain_id",
      "network",
      "block_height",
      "block_hash",
      "validator_set_hash",
      "canonical_head_height",
      "observed_at",
      "source",
    ],
    [],
    path,
  );
  if (value.schema_version !== 1) {
    fail(`${path}.schema_version is unsupported`);
  }
  requireObject(value.source, `${path}.source`);
  requireExactKeys(
    value.source,
    ["operator_id", "reference", "evidence_sha256"],
    [],
    `${path}.source`,
  );
  const anchor = {
    schema_version: 1,
    chain_id: requireString(value.chain_id, `${path}.chain_id`),
    network: requireString(value.network, `${path}.network`),
    block_height: requireSafeInteger(
      value.block_height,
      `${path}.block_height`,
    ),
    block_hash: requireNonZeroHash(value.block_hash, `${path}.block_hash`),
    validator_set_hash: requireLowerHash(
      value.validator_set_hash,
      `${path}.validator_set_hash`,
    ),
    canonical_head_height: requireSafeInteger(
      value.canonical_head_height,
      `${path}.canonical_head_height`,
    ),
    observed_at: requireUtcSeconds(value.observed_at, `${path}.observed_at`),
    source: {
      operator_id: requireString(
        value.source.operator_id,
        `${path}.source.operator_id`,
      ),
      reference: requireString(
        value.source.reference,
        `${path}.source.reference`,
      ),
      evidence_sha256: requireNonZeroHash(
        value.source.evidence_sha256,
        `${path}.source.evidence_sha256`,
      ),
    },
  };
  if (anchor.canonical_head_height < anchor.block_height) {
    fail(`${path}.canonical_head_height is below the checkpoint anchor`);
  }
  return anchor;
}

export function validateCanonicalAnchor(
  checkpointInput,
  anchorInput,
  maxAnchorAgeBlocks,
) {
  const checkpoint = normalizeCheckpoint(checkpointInput);
  const anchor = normalizeCanonicalAnchor(anchorInput);
  requireSafeInteger(maxAnchorAgeBlocks, "max_anchor_age_blocks");
  const expectedHeight = checkpoint.effective_from_height - 1;
  if (anchor.block_height !== expectedHeight) {
    fail(
      `canonical anchor height mismatch: supplied=${anchor.block_height} expected=${expectedHeight}`,
    );
  }
  if (anchor.block_hash !== checkpoint.previous_block_hash) {
    fail("canonical anchor conflicts with checkpoint previous_block_hash");
  }
  if (
    anchor.validator_set_hash !== checkpoint.previous_validator_set_hash
  ) {
    fail("canonical anchor conflicts with checkpoint prior validator-set hash");
  }
  const anchorAgeBlocks =
    anchor.canonical_head_height - anchor.block_height;
  if (anchorAgeBlocks > maxAnchorAgeBlocks) {
    fail(
      `checkpoint is outside the weak-subjectivity window: age=${anchorAgeBlocks} maximum=${maxAnchorAgeBlocks}`,
    );
  }
  return { anchor, anchorAgeBlocks };
}

function checkpointEnvJson(checkpoint) {
  // This is byte-for-byte the compact declaration-order JSON that Rust
  // serde_json emits for the normalized snapshot.
  const serialized = JSON.stringify(checkpoint);
  const byteLength = Buffer.byteLength(serialized, "utf8");
  if (byteLength > MAX_CHECKPOINT_ENVELOPE_BYTES) {
    fail(
      `normalized checkpoint is ${byteLength} bytes; the protocol envelope limit is ${MAX_CHECKPOINT_ENVELOPE_BYTES}`,
    );
  }
  return serialized;
}

function makeChecklist({ hasPredecessor }) {
  return [
    { id: "checkpoint-schema-and-exact-json", status: "pass" },
    { id: "next-validator-set-hash", status: "pass" },
    { id: "prior-validator-set-hash", status: "pass" },
    { id: "distinct-prior-set-quorum-signatures", status: "pass" },
    { id: "canonical-anchor-height", status: "pass" },
    { id: "canonical-anchor-block-hash", status: "pass" },
    { id: "canonical-anchor-prior-set-hash", status: "pass" },
    { id: "weak-subjectivity-window", status: "pass" },
    {
      id: "replay-rollback-predecessor",
      status: "pass",
      mode: hasPredecessor ? "checked" : "explicit-initial-release",
    },
  ];
}

function normalizeTrustedPublicKey(value, path) {
  return requireLowerHash(value, path);
}

function evidenceSigningBytes(payload) {
  return Buffer.from(
    `${CHECKPOINT_EVIDENCE_DOMAIN}\n${canonicalJson(payload)}`,
    "utf8",
  );
}

function evidenceDigest(envelope) {
  return sha256Hex(canonicalJson(envelope));
}

function readBoundedFile(path, label, maximumBytes, encoding = null) {
  let size;
  try {
    size = statSync(path).size;
  } catch (error) {
    fail(`cannot inspect ${label} at ${path}: ${error.message}`);
  }
  if (size > maximumBytes) {
    fail(`${label} at ${path} exceeds the ${maximumBytes}-byte input limit`);
  }
  try {
    return readFileSync(path, encoding ?? undefined);
  } catch (error) {
    fail(`cannot read ${label} at ${path}: ${error.message}`);
  }
}

function validateSuccession(candidate, predecessor) {
  if (candidate.chain_id !== predecessor.chain_id) {
    fail("predecessor evidence belongs to a different chain_id");
  }
  if (candidate.network !== predecessor.network) {
    fail("predecessor evidence belongs to a different network");
  }
  if (
    candidate.logical_transition_id === predecessor.logical_transition_id
  ) {
    fail("checkpoint publication is an exact replay of predecessor evidence");
  }
  if (
    candidate.checkpoint.effective_from_height ===
    predecessor.checkpoint.effective_from_height
  ) {
    fail("checkpoint conflicts with an already-published checkpoint height");
  }
  if (
    candidate.checkpoint.effective_from_height <
    predecessor.checkpoint.effective_from_height
  ) {
    fail("checkpoint publication would roll authority backward");
  }
  if (
    candidate.checkpoint.previous_validator_set_hash !==
    predecessor.checkpoint.validator_set_hash
  ) {
    fail(
      "checkpoint does not directly descend from the previously published authority",
    );
  }
}

function buildPayloadCore({
  checkpoint,
  previousValidatorSet,
  canonicalAnchor,
  maxAnchorAgeBlocks,
  verifierId,
  verifiedAt,
  releaseSequence,
  predecessorEvidenceSha256,
}) {
  const quorum = validateCheckpointQuorum(
    checkpoint,
    previousValidatorSet,
  );
  const anchorResult = validateCanonicalAnchor(
    quorum.checkpoint,
    canonicalAnchor,
    maxAnchorAgeBlocks,
  );
  requireString(verifierId, "verifier_id");
  requireUtcSeconds(verifiedAt, "verified_at");
  if (Date.parse(verifiedAt) < Date.parse(anchorResult.anchor.observed_at)) {
    fail("verified_at precedes the canonical-anchor observation");
  }
  requirePositiveSafeInteger(releaseSequence, "release_sequence");
  if (releaseSequence === 1) {
    if (predecessorEvidenceSha256 !== null) {
      fail("initial checkpoint evidence must not name a predecessor");
    }
  } else {
    requireLowerHash(
      predecessorEvidenceSha256,
      "predecessor_evidence_sha256",
    );
  }
  const envJson = checkpointEnvJson(quorum.checkpoint);
  return {
    domain: CHECKPOINT_EVIDENCE_DOMAIN,
    tool_version: TOOL_VERSION,
    chain_id: anchorResult.anchor.chain_id,
    network: anchorResult.anchor.network,
    verifier_id: verifierId,
    verified_at: verifiedAt,
    release_sequence: releaseSequence,
    predecessor_evidence_sha256: predecessorEvidenceSha256,
    checkpoint: quorum.checkpoint,
    checkpoint_env_json: envJson,
    checkpoint_env_sha256: sha256Hex(envJson),
    logical_transition_id: logicalTransitionId(quorum.checkpoint),
    witness_hash: witnessHash(quorum.checkpoint),
    transition_message: transitionMessage(quorum.checkpoint),
    previous_validator_set: quorum.previousSet,
    quorum: {
      prior_set_size: quorum.previousSet.validators.length,
      required: quorum.quorumRequired,
      valid: quorum.validSigners.length,
      valid_signers: quorum.validSigners,
    },
    canonical_anchor: anchorResult.anchor,
    canonical_anchor_sha256: sha256Hex(
      canonicalJson(anchorResult.anchor),
    ),
    weak_subjectivity_window: {
      canonical_head_height: anchorResult.anchor.canonical_head_height,
      anchor_age_blocks: anchorResult.anchorAgeBlocks,
      maximum_anchor_age_blocks: maxAnchorAgeBlocks,
    },
    checklist: makeChecklist({
      hasPredecessor: predecessorEvidenceSha256 !== null,
    }),
  };
}

export function validateEvidenceEnvelope(
  envelopeInput,
  trustedVerifierPublicKey,
) {
  const envelope = requireObject(envelopeInput, "evidence");
  requireExactKeys(
    envelope,
    ["schema_version", "kind", "payload", "signature"],
    [],
    "evidence",
  );
  if (envelope.schema_version !== CHECKPOINT_EVIDENCE_SCHEMA_VERSION) {
    fail("evidence.schema_version is unsupported");
  }
  if (envelope.kind !== CHECKPOINT_EVIDENCE_KIND) {
    fail("evidence.kind is unsupported");
  }
  const payload = requireObject(envelope.payload, "evidence.payload");
  const expectedPayload = buildPayloadCore({
    checkpoint: payload.checkpoint,
    previousValidatorSet: payload.previous_validator_set,
    canonicalAnchor: payload.canonical_anchor,
    maxAnchorAgeBlocks:
      payload.weak_subjectivity_window?.maximum_anchor_age_blocks,
    verifierId: payload.verifier_id,
    verifiedAt: payload.verified_at,
    releaseSequence: payload.release_sequence,
    predecessorEvidenceSha256: payload.predecessor_evidence_sha256,
  });
  if (canonicalJson(payload) !== canonicalJson(expectedPayload)) {
    fail("evidence payload contains inconsistent, missing, or unknown claims");
  }

  const signature = requireObject(envelope.signature, "evidence.signature");
  requireExactKeys(
    signature,
    ["algorithm", "public_key", "signature"],
    [],
    "evidence.signature",
  );
  if (signature.algorithm !== "ed25519") {
    fail("evidence.signature.algorithm must be ed25519");
  }
  const trusted = normalizeTrustedPublicKey(
    trustedVerifierPublicKey,
    "trusted_verifier_public_key",
  );
  const embedded = requireLowerHash(
    signature.public_key,
    "evidence.signature.public_key",
  );
  if (embedded !== trusted) {
    fail("evidence signer is not the externally trusted verifier");
  }
  const detached = requireLowerSignature(
    signature.signature,
    "evidence.signature.signature",
  );
  if (
    !verifyMessage(
      null,
      evidenceSigningBytes(expectedPayload),
      publicKeyFromRawHex(trusted, "trusted_verifier_public_key"),
      Buffer.from(detached, "hex"),
    )
  ) {
    fail("evidence detached signature is invalid");
  }
  return {
    envelope,
    payload: expectedPayload,
    evidenceSha256: evidenceDigest(envelope),
  };
}

export function buildCheckpointEvidence({
  checkpoint,
  previousValidatorSet,
  canonicalAnchor,
  maxAnchorAgeBlocks,
  verifierId,
  verifiedAt,
  evidencePrivateKey,
  initialRelease = false,
  predecessorEvidence = null,
  trustedPredecessorPublicKey = null,
}) {
  if (initialRelease === (predecessorEvidence !== null)) {
    fail(
      "select exactly one rollout mode: initialRelease or predecessorEvidence",
    );
  }

  let predecessor = null;
  let releaseSequence = 1;
  let predecessorEvidenceSha256 = null;
  if (predecessorEvidence !== null) {
    if (!trustedPredecessorPublicKey) {
      fail("predecessor evidence requires an externally trusted verifier key");
    }
    predecessor = validateEvidenceEnvelope(
      predecessorEvidence,
      trustedPredecessorPublicKey,
    );
    releaseSequence = predecessor.payload.release_sequence + 1;
    predecessorEvidenceSha256 = predecessor.evidenceSha256;
  }

  const payload = buildPayloadCore({
    checkpoint,
    previousValidatorSet,
    canonicalAnchor,
    maxAnchorAgeBlocks,
    verifierId,
    verifiedAt,
    releaseSequence,
    predecessorEvidenceSha256,
  });
  if (predecessor) {
    validateSuccession(payload, predecessor.payload);
  }

  let privateKey;
  try {
    privateKey = createPrivateKey(evidencePrivateKey);
  } catch (error) {
    fail(`cannot load evidence signing key: ${error.message}`);
  }
  if (privateKey.asymmetricKeyType !== "ed25519") {
    fail("evidence signing key must be Ed25519");
  }
  const publicDer = createPublicKey(privateKey).export({
    type: "spki",
    format: "der",
  });
  if (
    publicDer.length !== ED25519_SPKI_PREFIX.length + 32 ||
    !publicDer.subarray(0, ED25519_SPKI_PREFIX.length).equals(
      ED25519_SPKI_PREFIX,
    )
  ) {
    fail("evidence signing key has an unexpected Ed25519 encoding");
  }
  const publicKey = publicDer.subarray(-32).toString("hex");
  const signature = signMessage(
    null,
    evidenceSigningBytes(payload),
    privateKey,
  ).toString("hex");
  return {
    schema_version: CHECKPOINT_EVIDENCE_SCHEMA_VERSION,
    kind: CHECKPOINT_EVIDENCE_KIND,
    payload,
    signature: {
      algorithm: "ed25519",
      public_key: publicKey,
      signature,
    },
  };
}

export function validateEvidenceSuccession(
  currentEvidence,
  trustedCurrentPublicKey,
  predecessorEvidence,
  trustedPredecessorPublicKey,
) {
  const current = validateEvidenceEnvelope(
    currentEvidence,
    trustedCurrentPublicKey,
  );
  const predecessor = validateEvidenceEnvelope(
    predecessorEvidence,
    trustedPredecessorPublicKey,
  );
  if (
    current.payload.predecessor_evidence_sha256 !==
    predecessor.evidenceSha256
  ) {
    fail("evidence predecessor digest does not match the supplied record");
  }
  if (
    current.payload.release_sequence !==
    predecessor.payload.release_sequence + 1
  ) {
    fail("evidence release sequence is not continuous");
  }
  validateSuccession(current.payload, predecessor.payload);
  return current;
}

function readJson(path, label) {
  const raw = readBoundedFile(path, label, MAX_INPUT_FILE_BYTES, "utf8");
  try {
    const parsed = JSON.parse(raw);
    assertJsonNumbersAreSafe(parsed, label);
    return parsed;
  } catch (error) {
    if (error.message.startsWith("weak-subjectivity checkpoint gate:")) {
      throw error;
    }
    fail(`${label} at ${path} is not valid JSON: ${error.message}`);
  }
}

function parseArguments(argv) {
  const command = argv[0];
  const options = new Map();
  const flags = new Set();
  for (let index = 1; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--initial-release") {
      flags.add(argument);
      continue;
    }
    if (!argument.startsWith("--")) {
      fail(`unexpected positional argument ${argument}`);
    }
    const value = argv[index + 1];
    if (!value || value.startsWith("--")) {
      fail(`${argument} requires a value`);
    }
    if (options.has(argument)) {
      fail(`${argument} was specified more than once`);
    }
    options.set(argument, value);
    index += 1;
  }
  return { command, options, flags };
}

function requiredOption(options, name) {
  const value = options.get(name);
  if (!value) {
    fail(`${name} is required`);
  }
  return value;
}

function requireOnlyOptions(options, allowed, command) {
  const allowedSet = new Set(allowed);
  const unknown = [...options.keys()].filter((name) => !allowedSet.has(name));
  if (unknown.length > 0) {
    fail(`${command} contains unknown option(s): ${unknown.join(", ")}`);
  }
}

function parseUnsignedInteger(value, path) {
  if (!/^(?:0|[1-9][0-9]*)$/u.test(value)) {
    fail(`${path} must be a canonical unsigned integer`);
  }
  return requireSafeInteger(Number(value), path);
}

function usage() {
  console.log(`Usage:
  node scripts/verify-weak-subjectivity-checkpoint.mjs publish \\
    --checkpoint FILE --previous-validator-set FILE --canonical-anchor FILE \\
    --max-anchor-age-blocks N --verifier-id ID --verified-at RFC3339 \\
    --evidence-signing-key ED25519_PKCS8_PEM --output FILE \\
    (--initial-release | --predecessor-evidence FILE \\
      --trusted-predecessor-public-key LOWER_HEX)

  node scripts/verify-weak-subjectivity-checkpoint.mjs verify-evidence \\
    --evidence FILE --trusted-verifier-public-key LOWER_HEX \\
    [--predecessor-evidence FILE --trusted-predecessor-public-key LOWER_HEX]

The publish command validates the exact normalized checkpoint, prior-set
quorum, operator-supplied canonical anchor, freshness window, and
replay/rollback lineage before writing a signed evidence record. It never
creates a validator certificate, validator key, or canonical-chain anchor.`);
}

function runCli(argv) {
  const { command, options, flags } = parseArguments(argv);
  if (command === "--help" || command === "-h" || !command) {
    usage();
    return;
  }
  if (command === "publish") {
    requireOnlyOptions(
      options,
      [
        "--checkpoint",
        "--previous-validator-set",
        "--canonical-anchor",
        "--max-anchor-age-blocks",
        "--verifier-id",
        "--verified-at",
        "--evidence-signing-key",
        "--output",
        "--predecessor-evidence",
        "--trusted-predecessor-public-key",
      ],
      "publish",
    );
    const checkpoint = readJson(
      requiredOption(options, "--checkpoint"),
      "checkpoint",
    );
    const previousValidatorSet = readJson(
      requiredOption(options, "--previous-validator-set"),
      "previous validator set",
    );
    const canonicalAnchor = readJson(
      requiredOption(options, "--canonical-anchor"),
      "canonical anchor",
    );
    const predecessorPath = options.get("--predecessor-evidence");
    const initialRelease = flags.has("--initial-release");
    if (initialRelease === Boolean(predecessorPath)) {
      fail(
        "publish requires exactly one of --initial-release or --predecessor-evidence",
      );
    }
    if (
      initialRelease &&
      options.has("--trusted-predecessor-public-key")
    ) {
      fail(
        "--trusted-predecessor-public-key is invalid with --initial-release",
      );
    }
    const predecessorEvidence = predecessorPath
      ? readJson(predecessorPath, "predecessor evidence")
      : null;
    const signingKeyPath = requiredOption(
      options,
      "--evidence-signing-key",
    );
    const evidencePrivateKey = readBoundedFile(
      signingKeyPath,
      "evidence signing key",
      MAX_PRIVATE_KEY_BYTES,
    );
    const envelope = buildCheckpointEvidence({
      checkpoint,
      previousValidatorSet,
      canonicalAnchor,
      maxAnchorAgeBlocks: parseUnsignedInteger(
        requiredOption(options, "--max-anchor-age-blocks"),
        "--max-anchor-age-blocks",
      ),
      verifierId: requiredOption(options, "--verifier-id"),
      verifiedAt: requiredOption(options, "--verified-at"),
      evidencePrivateKey,
      initialRelease,
      predecessorEvidence,
      trustedPredecessorPublicKey: options.get(
        "--trusted-predecessor-public-key",
      ),
    });
    const output = requiredOption(options, "--output");
    writeFileSync(output, `${JSON.stringify(envelope, null, 2)}\n`, {
      encoding: "utf8",
      flag: "wx",
    });
    console.log(
      JSON.stringify({
        status: "verified-and-signed",
        evidence: output,
        verifier_public_key: envelope.signature.public_key,
        checkpoint_env_json: envelope.payload.checkpoint_env_json,
        checkpoint_env_sha256: envelope.payload.checkpoint_env_sha256,
        logical_transition_id: envelope.payload.logical_transition_id,
        witness_hash: envelope.payload.witness_hash,
      }),
    );
    return;
  }
  if (command === "verify-evidence") {
    requireOnlyOptions(
      options,
      [
        "--evidence",
        "--trusted-verifier-public-key",
        "--predecessor-evidence",
        "--trusted-predecessor-public-key",
      ],
      "verify-evidence",
    );
    const envelope = readJson(
      requiredOption(options, "--evidence"),
      "evidence",
    );
    const trusted = requiredOption(
      options,
      "--trusted-verifier-public-key",
    );
    const predecessorPath = options.get("--predecessor-evidence");
    if (
      !predecessorPath &&
      options.has("--trusted-predecessor-public-key")
    ) {
      fail(
        "--trusted-predecessor-public-key requires --predecessor-evidence",
      );
    }
    let result;
    if (predecessorPath) {
      result = validateEvidenceSuccession(
        envelope,
        trusted,
        readJson(predecessorPath, "predecessor evidence"),
        requiredOption(options, "--trusted-predecessor-public-key"),
      );
    } else {
      result = validateEvidenceEnvelope(envelope, trusted);
      if (result.payload.release_sequence !== 1) {
        fail(
          "non-initial evidence requires --predecessor-evidence for lineage verification",
        );
      }
    }
    console.log(
      JSON.stringify({
        status: "valid",
        evidence_sha256: result.evidenceSha256,
        logical_transition_id: result.payload.logical_transition_id,
        checkpoint_env_sha256: result.payload.checkpoint_env_sha256,
      }),
    );
    return;
  }
  fail(`unknown command ${command}`);
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === resolve(fileURLToPath(import.meta.url))) {
  try {
    runCli(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
