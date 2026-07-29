import {
  X509Certificate,
  createHash,
  createPublicKey,
  verify as verifySignature,
} from "node:crypto";
import {
  closeSync,
  constants as fsConstants,
  fstatSync,
  lstatSync,
  openSync,
  readSync,
} from "node:fs";
import { join } from "node:path";

import * as sigstoreBundle from "@sigstore/bundle";
import * as sigstoreModels from "@sigstore/protobuf-specs";
import * as sigstoreVerify from "@sigstore/verify";
import npa from "npm-package-arg";

import { inspectNpmTarball, MAX_CHALLENGE_BYTES } from "./archive.mjs";
import { Rejected, reject } from "./errors.mjs";
import { canonicalize, parseJsonStrict } from "./strict-json.mjs";

export const PROTOCOL_VERSION = 2;
export const AUTHORITY = "npm_trusted_publisher";
export const AUDIENCE = "npm:registry.npmjs.org";
export const REGISTRY_ORIGIN = "https://registry.npmjs.org";
export const PUBLISH_PREDICATE =
  "https://github.com/npm/attestation/tree/main/specs/publish/v0.1";
export const PUBLISH_STATEMENT = "https://in-toto.io/Statement/v0.1";
export const DSSE_PAYLOAD_TYPE = "application/vnd.in-toto+json";
export const POLICY_FILENAME = "npm-policy.json";
export const TRUST_ROOT_FILENAME = "npm-trusted-root.json";
export const REGISTRY_KEYS_FILENAME = "npm-registry-keys.json";
export const AUTH_MODE_ERROR =
  "ERR_NPM_TRUSTED_PUBLISHING_AUTH_MODE_UNPROVEN";

const MAX_STDIN_BYTES = 146 * 1024 * 1024;
const MAX_EVIDENCE_BYTES = 8 * 1024 * 1024;
const MAX_ARTIFACT_BYTES = 64 * 1024 * 1024;
const MAX_POLICY_BYTES = 1024 * 1024;
const MAX_TRUST_ROOT_BYTES = 4 * 1024 * 1024;
const MAX_REGISTRY_KEYS_BYTES = 1024 * 1024;
const MAX_CHALLENGE_TTL_SECONDS = 15 * 60;
const MAX_CLOCK_SKEW_SECONDS = 60;
const MAX_SAFE_INTEGER = Number.MAX_SAFE_INTEGER;

const SHA256 = /^[0-9a-f]{64}$/;
const SHA512 = /^[0-9a-f]{128}$/;
const ED25519_KEY = /^[0-9a-f]{64}$/;
const ED25519_SIGNATURE = /^[0-9a-f]{128}$/;
const EVM_ADDRESS = /^0x[0-9a-f]{40}$/;
const BASE64 = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;
const KEY_ID = /^SHA256:[A-Za-z0-9+/]{43}$/;
const RFC3339_UTC =
  /^(?:[0-9]{4})-(?:0[1-9]|1[0-2])-(?:0[1-9]|[12][0-9]|3[01])T(?:[01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](?:\.[0-9]{1,9})?Z$/;

const INVOCATION_KEYS = [
  "adapter_id",
  "adapter_version",
  "policy_sha256",
  "request",
  "schema_version",
  "trust_root_sha256",
];
const REQUEST_KEYS = [
  "artifact_digest",
  "artifact_digest_algorithm",
  "artifact_hex",
  "artifact_url",
  "authority",
  "challenge",
  "evidence_hex",
  "evidence_sha256",
  "evidence_url",
  "expected_audience",
  "expected_issuer",
  "expected_subject",
  "schema_version",
];
const CHALLENGE_KEYS = [
  "chain_id",
  "challenge_document",
  "current_controller_ed25519_pubkey",
  "current_controller_evm_address",
  "expires_at",
  "genesis_hash",
  "issued_at",
  "namespace",
  "next_controller_ed25519_pubkey",
  "next_controller_evm_address",
  "next_controller_signature",
  "nonce",
  "purpose",
  "schema_version",
];
const POLICY_KEYS = [
  "adapter_id",
  "adapter_version",
  "archive",
  "authentication_mode",
  "authority",
  "publish",
  "publishers",
  "registry_keys_sha256",
  "registry_origin",
  "schema_version",
  "sigstore",
];
const ARCHIVE_KEYS = [
  "challenge_prefix",
  "max_compression_ratio",
  "max_entries",
  "max_entry_bytes",
  "max_uncompressed_bytes",
  "package_manifest_path",
];
const PUBLISHER_KEYS = [
  "issuer",
  "namespace",
  "provenance_predicate_type",
  "provenance_statement_type",
  "subject",
];
const SUPPORTED_PROVENANCE = new Map([
  ["https://slsa.dev/provenance/v0.2", "https://in-toto.io/Statement/v0.1"],
  ["https://slsa.dev/provenance/v1", "https://in-toto.io/Statement/v1"],
]);
const SUPPORTED_BUNDLE_MEDIA_TYPES = new Set([
  "application/vnd.dev.sigstore.bundle+json;version=0.1",
  "application/vnd.dev.sigstore.bundle+json;version=0.2",
  "application/vnd.dev.sigstore.bundle.v0.3+json",
]);

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function requireObject(value, label, keys) {
  if (!isObject(value)) {
    reject(`${label} must be an object`);
  }
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (
    actual.length !== expected.length ||
    actual.some((key, index) => key !== expected[index])
  ) {
    reject(`${label} has missing or unknown fields`);
  }
  return value;
}

function requireObjectOptional(value, label, required, optional) {
  if (!isObject(value)) {
    reject(`${label} must be an object`);
  }
  const present = new Set(Object.keys(value));
  if (
    required.some((key) => !present.has(key)) ||
    [...present].some((key) => !required.includes(key) && !optional.includes(key))
  ) {
    reject(`${label} has missing or unknown fields`);
  }
  return value;
}

function requireString(value, label, maximum) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    Buffer.byteLength(value, "utf8") > maximum ||
    value !== value.trim() ||
    [...value].some(
      (character) =>
        /\p{C}/u.test(character) ||
        (/\s/u.test(character) && character !== " "),
    )
  ) {
    reject(`${label} is not canonical text`);
  }
  return value;
}

function requireSafeInteger(value, label, minimum, maximum = MAX_SAFE_INTEGER) {
  if (
    !Number.isSafeInteger(value) ||
    value < minimum ||
    value > maximum
  ) {
    reject(`${label} is outside its permitted integer range`);
  }
  return value;
}

function requireSha256(value, label) {
  if (typeof value !== "string" || !SHA256.test(value)) {
    reject(`${label} must be a lowercase SHA-256 digest`);
  }
  return value;
}

function requireSha512(value, label) {
  if (typeof value !== "string" || !SHA512.test(value)) {
    reject(`${label} must be a lowercase SHA-512 digest`);
  }
  return value;
}

function digest(algorithm, bytes) {
  return createHash(algorithm).update(bytes).digest("hex");
}

function decodeHex(value, label, maximum) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length % 2 !== 0 ||
    value.length / 2 > maximum ||
    !/^[0-9a-f]+$/.test(value)
  ) {
    reject(`${label} is not bounded lowercase hexadecimal`);
  }
  return Buffer.from(value, "hex");
}

function decodeBase64(value, label, maximum = 8 * 1024 * 1024) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length % 4 !== 0 ||
    !BASE64.test(value)
  ) {
    reject(`${label} is not canonical base64`);
  }
  const bytes = Buffer.from(value, "base64");
  if (bytes.length > maximum || bytes.toString("base64") !== value) {
    reject(`${label} is invalid or exceeds its decoded limit`);
  }
  return bytes;
}

function canonicalNpmName(value, label = "npm package name") {
  const name = requireString(value, label, 214);
  if (name !== name.toLowerCase()) {
    reject(`${label} must be lowercase`);
  }
  const component = /^[a-z0-9._~-]+$/;
  if (name.startsWith("@")) {
    const scoped = name.slice(1);
    const separator = scoped.indexOf("/");
    if (
      separator <= 0 ||
      separator !== scoped.lastIndexOf("/") ||
      !component.test(scoped.slice(0, separator)) ||
      !component.test(scoped.slice(separator + 1))
    ) {
      reject(`${label} is not a canonical scoped npm name`);
    }
  } else if (name.includes("/") || !component.test(name)) {
    reject(`${label} is not a canonical npm name`);
  }
  let parsed;
  try {
    parsed = npa(`${name}@0.0.0`);
  } catch (error) {
    reject(`${label} is not accepted by npm-package-arg`, undefined, {
      cause: error,
    });
  }
  if (
    parsed.name !== name ||
    parsed.type !== "version" ||
    parsed.fetchSpec !== "0.0.0"
  ) {
    reject(`${label} is ambiguous to npm-package-arg`);
  }
  return name;
}

function canonicalNamespace(value) {
  const namespace = requireString(value, "challenge.namespace", 512);
  if (!namespace.startsWith("npm:") || !namespace.endsWith("@")) {
    reject("challenge namespace is not an npm namespace");
  }
  const name = namespace.slice(4, -1);
  canonicalNpmName(name, "challenge namespace package");
  if (namespace !== `npm:${name}@`) {
    reject("challenge namespace is not canonical");
  }
  return namespace;
}

function authorityIntent(challenge) {
  return Buffer.from(
    [
      "creg-namespace-authority-intent-v1",
      "service:creg-package-ownership",
      `purpose:${challenge.purpose}`,
      `document:${challenge.challenge_document}`,
      `chain_id:${challenge.chain_id}`,
      `genesis_hash:${challenge.genesis_hash}`,
      `namespace:${challenge.namespace}`,
      `current_ed25519_pubkey:${challenge.current_controller_ed25519_pubkey}`,
      `current_evm_address:${challenge.current_controller_evm_address}`,
      `next_ed25519_pubkey:${challenge.next_controller_ed25519_pubkey}`,
      `next_evm_address:${challenge.next_controller_evm_address}`,
      `nonce:${challenge.nonce}`,
      `issued_at:${challenge.issued_at}`,
      `expires_at:${challenge.expires_at}`,
    ].join("\n"),
    "utf8",
  );
}

export function validateChallenge(value, now) {
  const challenge = requireObject(value, "request.challenge", CHALLENGE_KEYS);
  if (challenge.schema_version !== 1) {
    reject("challenge schema version is unsupported");
  }
  if (challenge.purpose !== "claim" && challenge.purpose !== "recover") {
    reject("challenge purpose is unsupported");
  }
  const document =
    challenge.purpose === "claim"
      ? "creg-claim.json"
      : "creg-recovery.json";
  if (challenge.challenge_document !== document) {
    reject("challenge document does not match its purpose");
  }
  requireString(challenge.chain_id, "challenge.chain_id", 128);
  requireSha256(challenge.genesis_hash, "challenge.genesis_hash");
  canonicalNamespace(challenge.namespace);
  if (
    typeof challenge.next_controller_ed25519_pubkey !== "string" ||
    !ED25519_KEY.test(challenge.next_controller_ed25519_pubkey) ||
    typeof challenge.next_controller_evm_address !== "string" ||
    !EVM_ADDRESS.test(challenge.next_controller_evm_address) ||
    typeof challenge.next_controller_signature !== "string" ||
    !ED25519_SIGNATURE.test(challenge.next_controller_signature)
  ) {
    reject("challenge next-controller material is invalid");
  }
  if (challenge.purpose === "claim") {
    if (
      challenge.current_controller_ed25519_pubkey !== "" ||
      challenge.current_controller_evm_address !== ""
    ) {
      reject("claim challenge unexpectedly names a current controller");
    }
  } else if (
    typeof challenge.current_controller_ed25519_pubkey !== "string" ||
    !ED25519_KEY.test(challenge.current_controller_ed25519_pubkey) ||
    typeof challenge.current_controller_evm_address !== "string" ||
    !EVM_ADDRESS.test(challenge.current_controller_evm_address)
  ) {
    reject("recovery challenge current-controller material is invalid");
  }
  requireSafeInteger(challenge.nonce, "challenge.nonce", 1);
  requireSafeInteger(challenge.issued_at, "challenge.issued_at", 0);
  requireSafeInteger(challenge.expires_at, "challenge.expires_at", 1);
  if (
    challenge.expires_at <= challenge.issued_at ||
    challenge.expires_at - challenge.issued_at >
      MAX_CHALLENGE_TTL_SECONDS ||
    now >= challenge.expires_at ||
    challenge.issued_at > now + MAX_CLOCK_SKEW_SECONDS
  ) {
    reject("challenge is expired, premature, or too long-lived");
  }

  const intent = authorityIntent(challenge);
  let key;
  try {
    const rawKey = Buffer.from(challenge.next_controller_ed25519_pubkey, "hex");
    const spkiPrefix = Buffer.from("302a300506032b6570032100", "hex");
    key = createPublicKey({
      key: Buffer.concat([spkiPrefix, rawKey]),
      format: "der",
      type: "spki",
    });
  } catch (error) {
    reject("challenge next-controller Ed25519 key is invalid", undefined, {
      cause: error,
    });
  }
  if (
    !verifySignature(
      null,
      intent,
      key,
      Buffer.from(challenge.next_controller_signature, "hex"),
    )
  ) {
    reject("challenge next-controller signature verification failed");
  }
  return {
    challenge,
    authorityIntentSha256: digest("sha256", intent),
  };
}

export function validateInvocation(raw, now) {
  const invocation = requireObject(
    parseJsonStrict(raw, "invocation", {
      maxBytes: MAX_STDIN_BYTES,
      maxDepth: 64,
    }),
    "invocation",
    INVOCATION_KEYS,
  );
  if (invocation.schema_version !== PROTOCOL_VERSION) {
    reject("invocation schema version is unsupported");
  }
  requireString(invocation.adapter_id, "adapter_id", 128);
  requireString(invocation.adapter_version, "adapter_version", 64);
  requireSha256(invocation.policy_sha256, "policy_sha256");
  requireSha256(invocation.trust_root_sha256, "trust_root_sha256");

  const request = requireObject(invocation.request, "request", REQUEST_KEYS);
  if (
    request.schema_version !== PROTOCOL_VERSION ||
    request.authority !== AUTHORITY
  ) {
    reject("request is not an npm authority-adapter v2 request");
  }
  if (request.artifact_digest_algorithm !== "sha512") {
    reject("npm artifact digest algorithm must be SHA-512");
  }
  const artifactDigest = requireSha512(
    request.artifact_digest,
    "artifact_digest",
  );
  const evidenceDigest = requireSha256(
    request.evidence_sha256,
    "evidence_sha256",
  );
  const evidence = decodeHex(
    request.evidence_hex,
    "evidence_hex",
    MAX_EVIDENCE_BYTES,
  );
  const artifact = decodeHex(
    request.artifact_hex,
    "artifact_hex",
    MAX_ARTIFACT_BYTES,
  );
  if (digest("sha256", evidence) !== evidenceDigest) {
    reject("exact evidence bytes do not match evidence_sha256");
  }
  if (digest("sha512", artifact) !== artifactDigest) {
    reject("exact artifact bytes do not match npm SHA-512");
  }
  requireString(request.expected_issuer, "expected_issuer", 512);
  requireString(request.expected_subject, "expected_subject", 1024);
  if (request.expected_audience !== AUDIENCE) {
    reject("expected audience is not the npm registry audience");
  }
  const validatedChallenge = validateChallenge(request.challenge, now);
  return {
    invocation,
    request,
    evidence,
    artifact,
    ...validatedChallenge,
  };
}

function fileIdentity(stat) {
  return [
    stat.dev,
    stat.ino,
    stat.mode,
    stat.size,
    stat.mtimeNs,
    stat.ctimeNs,
  ].join(":");
}

function readRegularFile(path, maximum, label) {
  let before;
  try {
    before = lstatSync(path, { bigint: true });
  } catch (error) {
    reject(`${label} is unavailable`, undefined, { cause: error });
  }
  if (
    !before.isFile() ||
    before.isSymbolicLink() ||
    before.size <= 0n ||
    before.size > BigInt(maximum)
  ) {
    reject(`${label} must be a bounded, non-symlink regular file`);
  }
  let descriptor;
  let bytes;
  let during;
  try {
    const noFollow = fsConstants.O_NOFOLLOW ?? 0;
    descriptor = openSync(path, fsConstants.O_RDONLY | noFollow);
    during = fstatSync(descriptor, { bigint: true });
    bytes = Buffer.alloc(Number(during.size));
    let offset = 0;
    while (offset < bytes.length) {
      const count = readSync(
        descriptor,
        bytes,
        offset,
        bytes.length - offset,
        offset,
      );
      if (count === 0) {
        break;
      }
      offset += count;
    }
    if (offset !== bytes.length) {
      reject(`${label} was truncated while it was read`);
    }
  } catch (error) {
    if (error instanceof Rejected) {
      throw error;
    }
    reject(`${label} could not be read`, undefined, { cause: error });
  } finally {
    if (descriptor !== undefined) {
      closeSync(descriptor);
    }
  }
  let after;
  try {
    after = lstatSync(path, { bigint: true });
  } catch (error) {
    reject(`${label} changed while it was read`, undefined, { cause: error });
  }
  if (
    !during.isFile() ||
    bytes.length > maximum ||
    fileIdentity(before) !== fileIdentity(during) ||
    fileIdentity(before) !== fileIdentity(after)
  ) {
    reject(`${label} changed or exceeded its limit while it was read`);
  }
  return bytes;
}

function loadPinnedFile(directory, filename, expectedSha256, maximum, label) {
  const bytes = readRegularFile(join(directory, filename), maximum, label);
  if (digest("sha256", bytes) !== expectedSha256) {
    reject(`${label} does not match its hash pin`);
  }
  return bytes;
}

function validateArchivePolicy(value) {
  const archive = requireObject(value, "policy.archive", ARCHIVE_KEYS);
  if (
    archive.challenge_prefix !== "package" ||
    archive.package_manifest_path !== "package/package.json"
  ) {
    reject("policy archive paths are not the canonical npm package paths");
  }
  requireSafeInteger(archive.max_entries, "archive.max_entries", 1, 100_000);
  requireSafeInteger(
    archive.max_entry_bytes,
    "archive.max_entry_bytes",
    MAX_CHALLENGE_BYTES,
    64 * 1024 * 1024,
  );
  requireSafeInteger(
    archive.max_uncompressed_bytes,
    "archive.max_uncompressed_bytes",
    archive.max_entry_bytes,
    512 * 1024 * 1024,
  );
  requireSafeInteger(
    archive.max_compression_ratio,
    "archive.max_compression_ratio",
    1,
    1_000,
  );
  return archive;
}

export function validatePolicy(raw, invocation) {
  const policy = requireObject(
    parseJsonStrict(raw, "npm authority policy", {
      maxBytes: MAX_POLICY_BYTES,
    }),
    "policy",
    POLICY_KEYS,
  );
  if (!canonicalize(policy).equals(raw)) {
    reject("npm authority policy is not canonical RFC 8785 JSON");
  }
  if (
    policy.schema_version !== 1 ||
    policy.authority !== AUTHORITY ||
    policy.adapter_id !== invocation.adapter_id ||
    policy.adapter_version !== invocation.adapter_version ||
    policy.registry_origin !== REGISTRY_ORIGIN
  ) {
    reject("npm authority policy does not match the pinned invocation");
  }
  requireSha256(policy.registry_keys_sha256, "policy.registry_keys_sha256");
  requireObject(policy.publish, "policy.publish", [
    "predicate_type",
    "statement_type",
  ]);
  if (
    policy.publish.predicate_type !== PUBLISH_PREDICATE ||
    policy.publish.statement_type !== PUBLISH_STATEMENT
  ) {
    reject("policy does not pin the official npm publish attestation format");
  }
  const sigstore = requireObject(policy.sigstore, "policy.sigstore", [
    "bundle_media_types",
    "instance",
  ]);
  if (sigstore.instance !== "production") {
    reject("policy does not require the Sigstore production instance");
  }
  if (
    !Array.isArray(sigstore.bundle_media_types) ||
    sigstore.bundle_media_types.length === 0 ||
    sigstore.bundle_media_types.length > SUPPORTED_BUNDLE_MEDIA_TYPES.size
  ) {
    reject("policy Sigstore bundle media types are empty or excessive");
  }
  const mediaTypes = new Set();
  for (const [index, mediaType] of sigstore.bundle_media_types.entries()) {
    requireString(mediaType, `policy.sigstore.bundle_media_types[${index}]`, 128);
    if (!SUPPORTED_BUNDLE_MEDIA_TYPES.has(mediaType) || mediaTypes.has(mediaType)) {
      reject("policy contains an unsupported or duplicate bundle media type");
    }
    mediaTypes.add(mediaType);
  }
  if (
    [...mediaTypes].sort().join("\n") !==
    [...sigstore.bundle_media_types].sort().join("\n")
  ) {
    reject("policy bundle media types are invalid");
  }
  const authenticationMode = requireObject(
    policy.authentication_mode,
    "policy.authentication_mode",
    ["official_cryptographic_signal", "required", "status"],
  );
  if (
    authenticationMode.required !== "trusted_publishing_oidc" ||
    authenticationMode.status !== "unavailable" ||
    authenticationMode.official_cryptographic_signal !== null
  ) {
    reject("policy attempts to bypass the npm authentication-mode gate");
  }
  validateArchivePolicy(policy.archive);
  if (
    !Array.isArray(policy.publishers) ||
    policy.publishers.length === 0 ||
    policy.publishers.length > 10_000
  ) {
    reject("policy.publishers must be a bounded non-empty list");
  }
  const identities = new Set();
  for (const [index, entryValue] of policy.publishers.entries()) {
    const entry = requireObject(
      entryValue,
      `policy.publishers[${index}]`,
      PUBLISHER_KEYS,
    );
    canonicalNamespace(entry.namespace);
    requireString(entry.issuer, `policy.publishers[${index}].issuer`, 512);
    requireString(entry.subject, `policy.publishers[${index}].subject`, 1024);
    if (
      SUPPORTED_PROVENANCE.get(entry.provenance_predicate_type) !==
      entry.provenance_statement_type
    ) {
      reject("policy publisher pins an unsupported provenance format pair");
    }
    const identity = canonicalize(entry).toString("utf8");
    if (identities.has(identity)) {
      reject("policy contains a duplicate publisher identity");
    }
    identities.add(identity);
  }
  return policy;
}

export function parsePackageIdentity(request, challenge) {
  const evidenceUrl = requireString(request.evidence_url, "evidence_url", 4096);
  const prefix = `${REGISTRY_ORIGIN}/-/npm/v1/attestations/`;
  if (!evidenceUrl.startsWith(prefix)) {
    reject("evidence URL is not the official npm attestation endpoint");
  }
  const suffix = evidenceUrl.slice(prefix.length);
  const versionSeparator = suffix.lastIndexOf("@");
  if (
    versionSeparator <= 0 ||
    suffix.includes("?") ||
    suffix.includes("#") ||
    suffix.includes("\\") ||
    suffix.includes("//")
  ) {
    reject("evidence URL does not identify one exact npm version");
  }
  const escapedName = suffix.slice(0, versionSeparator);
  const version = suffix.slice(versionSeparator + 1);
  let packageName;
  try {
    packageName = decodeURIComponent(escapedName.replace(/%2f/gi, "/"));
  } catch (error) {
    reject("evidence URL package name is not valid percent-encoding", undefined, {
      cause: error,
    });
  }
  canonicalNpmName(packageName);
  let spec;
  try {
    spec = npa(`${packageName}@${version}`);
  } catch (error) {
    reject("evidence URL is not an exact npm package version", undefined, {
      cause: error,
    });
  }
  if (
    spec.type !== "version" ||
    spec.registry !== true ||
    spec.name !== packageName ||
    spec.rawSpec !== version ||
    spec.fetchSpec !== version
  ) {
    reject("evidence URL package version is non-canonical or ambiguous");
  }
  const expectedEvidence =
    `${prefix}${spec.escapedName}@${version}`;
  if (evidenceUrl !== expectedEvidence) {
    reject("evidence URL is not npm-package-arg canonical");
  }
  if (challenge.namespace !== `npm:${packageName}@`) {
    reject("evidence package does not match the challenge namespace");
  }
  const tarballName = packageName.startsWith("@")
    ? packageName.slice(packageName.indexOf("/") + 1)
    : packageName;
  const expectedArtifact =
    `${REGISTRY_ORIGIN}/${packageName}/-/${tarballName}-${version}.tgz`;
  if (request.artifact_url !== expectedArtifact) {
    reject("artifact URL does not bind the same exact npm package and version");
  }
  return {
    packageName,
    packageVersion: version,
    purl: npa.toPurl(spec),
  };
}

export function validateStatementSubject(
  statement,
  label,
  identity,
  expectedType,
  expectedPredicate,
) {
  requireObject(statement, label, [
    "_type",
    "predicate",
    "predicateType",
    "subject",
  ]);
  if (
    statement._type !== expectedType ||
    statement.predicateType !== expectedPredicate
  ) {
    reject(`${label} has an unexpected statement or predicate type`);
  }
  if (!Array.isArray(statement.subject) || statement.subject.length !== 1) {
    reject(`${label} must contain exactly one subject`);
  }
  const subject = requireObject(statement.subject[0], `${label}.subject[0]`, [
    "digest",
    "name",
  ]);
  const subjectDigest = requireObject(
    subject.digest,
    `${label}.subject[0].digest`,
    ["sha512"],
  );
  if (
    subject.name !== identity.purl ||
    requireSha512(subjectDigest.sha512, `${label} subject SHA-512`) !==
      identity.artifactDigest
  ) {
    reject(`${label} does not bind the exact PURL and npm SHA-512`);
  }
}

function validateTransparency(bundle, label) {
  const material = requireObjectOptional(
    bundle.verificationMaterial,
    `${label}.verificationMaterial`,
    ["tlogEntries"],
    [
      "certificate",
      "publicKey",
      "timestampVerificationData",
      "x509CertificateChain",
    ],
  );
  const keyKinds = ["certificate", "publicKey", "x509CertificateChain"].filter(
    (key) => Object.hasOwn(material, key),
  );
  if (keyKinds.length !== 1) {
    reject(`${label} must contain exactly one signing-key representation`);
  }
  if (!Array.isArray(material.tlogEntries) || material.tlogEntries.length !== 1) {
    reject(`${label} must contain exactly one transparency-log entry`);
  }
  const entry = material.tlogEntries[0];
  if (!isObject(entry)) {
    reject(`${label} transparency entry must be an object`);
  }
  const integrated = Number(entry.integratedTime);
  if (
    typeof entry.integratedTime !== "string" ||
    !/^[1-9][0-9]*$/.test(entry.integratedTime) ||
    !Number.isSafeInteger(integrated)
  ) {
    reject(`${label} transparency integrated time is invalid`);
  }
  if (
    !isObject(entry.inclusionProof) ||
    !Array.isArray(entry.inclusionProof.hashes) ||
    !isObject(entry.inclusionProof.checkpoint) ||
    typeof entry.inclusionProof.checkpoint.envelope !== "string" ||
    entry.inclusionProof.checkpoint.envelope.length === 0 ||
    typeof entry.canonicalizedBody !== "string" ||
    entry.canonicalizedBody.length === 0
  ) {
    reject(`${label} lacks an offline inclusion proof and checkpoint`);
  }
  return { keyKind: keyKinds[0], integratedTime: integrated };
}

function decodeBundleAttestation(item, index, identity, policyEntry, policy) {
  const attestation = requireObjectOptional(
    item,
    `evidence.attestations[${index}]`,
    ["bundle", "predicateType"],
    ["signedAccessSignatureUrl"],
  );
  if (
    Object.hasOwn(attestation, "signedAccessSignatureUrl") &&
    attestation.signedAccessSignatureUrl !== ""
  ) {
    reject("npm evidence unexpectedly contains signed-access indirection");
  }
  requireString(attestation.predicateType, "attestation predicateType", 512);
  const bundle = requireObject(
    attestation.bundle,
    `evidence.attestations[${index}].bundle`,
    ["dsseEnvelope", "mediaType", "verificationMaterial"],
  );
  if (!policy.sigstore.bundle_media_types.includes(bundle.mediaType)) {
    reject("Sigstore bundle media type is not policy-pinned");
  }
  const envelope = requireObject(
    bundle.dsseEnvelope,
    `evidence.attestations[${index}].bundle.dsseEnvelope`,
    ["payload", "payloadType", "signatures"],
  );
  if (
    envelope.payloadType !== DSSE_PAYLOAD_TYPE ||
    !Array.isArray(envelope.signatures) ||
    envelope.signatures.length !== 1
  ) {
    reject("attestation is not a single-signature in-toto DSSE envelope");
  }
  const signature = requireObject(
    envelope.signatures[0],
    "DSSE signature",
    ["keyid", "sig"],
  );
  decodeBase64(signature.sig, "DSSE signature", 1024);
  if (typeof signature.keyid !== "string") {
    reject("DSSE keyid is not a string");
  }
  const payload = decodeBase64(envelope.payload, "DSSE payload", 2 * 1024 * 1024);
  const statement = parseJsonStrict(payload, "DSSE in-toto statement", {
    maxBytes: 2 * 1024 * 1024,
  });
  if (statement.predicateType !== attestation.predicateType) {
    reject("unsigned evidence predicateType disagrees with the signed statement");
  }
  const transparency = validateTransparency(
    bundle,
    `evidence.attestations[${index}].bundle`,
  );

  let kind;
  if (attestation.predicateType === PUBLISH_PREDICATE) {
    kind = "publish";
    validateStatementSubject(
      statement,
      "npm publish statement",
      identity,
      PUBLISH_STATEMENT,
      PUBLISH_PREDICATE,
    );
    const predicate = requireObject(
      statement.predicate,
      "npm publish predicate",
      ["name", "registry", "version"],
    );
    if (
      predicate.name !== identity.packageName ||
      predicate.version !== identity.packageVersion ||
      predicate.registry !== REGISTRY_ORIGIN
    ) {
      reject("npm publish predicate does not bind package, version, and registry");
    }
    if (
      transparency.keyKind !== "publicKey" ||
      !KEY_ID.test(signature.keyid)
    ) {
      reject("npm publish attestation is not signed by a named registry key");
    }
  } else if (
    attestation.predicateType === policyEntry.provenance_predicate_type
  ) {
    kind = "provenance";
    validateStatementSubject(
      statement,
      "npm provenance statement",
      identity,
      policyEntry.provenance_statement_type,
      policyEntry.provenance_predicate_type,
    );
    if (!isObject(statement.predicate)) {
      reject("npm provenance predicate is not an object");
    }
    if (
      !["certificate", "x509CertificateChain"].includes(
        transparency.keyKind,
      ) ||
      signature.keyid !== ""
    ) {
      reject("npm provenance is not a keyless certificate-signed bundle");
    }
  } else {
    reject("npm evidence contains an unpinned attestation predicate");
  }

  let parsedBundle;
  try {
    parsedBundle = sigstoreBundle.bundleFromJSON(bundle);
  } catch (error) {
    reject("attestation is not a valid Sigstore bundle", undefined, {
      cause: error,
    });
  }
  return {
    kind,
    bundle,
    parsedBundle,
    statement,
    signature,
    ...transparency,
  };
}

export function validateEvidence(raw, identity, policyEntry, policy) {
  const evidence = requireObject(
    parseJsonStrict(raw, "npm attestation evidence", {
      maxBytes: MAX_EVIDENCE_BYTES,
      maxDepth: 128,
    }),
    "evidence",
    ["attestations"],
  );
  if (
    !Array.isArray(evidence.attestations) ||
    evidence.attestations.length !== 2
  ) {
    reject("npm evidence must contain exactly one publish and one provenance attestation");
  }
  const decoded = evidence.attestations.map((item, index) =>
    decodeBundleAttestation(item, index, identity, policyEntry, policy),
  );
  const publish = decoded.filter((item) => item.kind === "publish");
  const provenance = decoded.filter((item) => item.kind === "provenance");
  if (publish.length !== 1 || provenance.length !== 1) {
    reject("npm evidence lacks a unique publish/provenance attestation pair");
  }
  return { evidence, publish: publish[0], provenance: provenance[0] };
}

export function validateRegistryKeys(raw) {
  const value = requireObject(
    parseJsonStrict(raw, "npm registry key material", {
      maxBytes: MAX_REGISTRY_KEYS_BYTES,
    }),
    "npm registry key material",
    ["keys"],
  );
  if (!Array.isArray(value.keys) || value.keys.length === 0 || value.keys.length > 64) {
    reject("npm registry key material is empty or excessive");
  }
  const result = new Map();
  for (const [index, item] of value.keys.entries()) {
    const key = requireObject(item, `npm registry key[${index}]`, [
      "expires",
      "key",
      "keyid",
      "keytype",
      "scheme",
    ]);
    if (
      !KEY_ID.test(key.keyid) ||
      key.keytype !== "ecdsa-sha2-nistp256" ||
      key.scheme !== "ecdsa-sha2-nistp256" ||
      result.has(key.keyid)
    ) {
      reject("npm registry key has unsupported or duplicate metadata");
    }
    let expires = null;
    if (key.expires !== null) {
      if (typeof key.expires !== "string" || !RFC3339_UTC.test(key.expires)) {
        reject("npm registry key expiry is not canonical UTC RFC 3339");
      }
      expires = new Date(key.expires);
      if (Number.isNaN(expires.valueOf())) {
        reject("npm registry key expiry is invalid");
      }
    }
    const der = decodeBase64(key.key, "npm registry public key", 4096);
    let publicKey;
    try {
      publicKey = createPublicKey({ key: der, format: "der", type: "spki" });
    } catch (error) {
      reject("npm registry public key is not valid SPKI", undefined, {
        cause: error,
      });
    }
    if (
      publicKey.asymmetricKeyType !== "ec" ||
      publicKey.asymmetricKeyDetails?.namedCurve !== "prime256v1"
    ) {
      reject("npm registry public key is not ECDSA P-256");
    }
    result.set(key.keyid, { expires, publicKey });
  }
  return result;
}

export function validateTrustedRoot(raw) {
  const value = parseJsonStrict(raw, "Sigstore trusted root", {
    maxBytes: MAX_TRUST_ROOT_BYTES,
    maxDepth: 128,
  });
  let root;
  try {
    root = sigstoreModels.TrustedRoot.fromJSON(value);
  } catch (error) {
    reject("pinned Sigstore trusted root is invalid", undefined, {
      cause: error,
    });
  }
  if (
    !Array.isArray(root.certificateAuthorities) ||
    root.certificateAuthorities.length === 0 ||
    !Array.isArray(root.tlogs) ||
    root.tlogs.length === 0 ||
    !Array.isArray(root.ctlogs) ||
    root.ctlogs.length === 0
  ) {
    reject("pinned Sigstore trusted root lacks production verification material");
  }
  return root;
}

function certificateFromBundle(bundle) {
  const content = bundle.verificationMaterial.content;
  let bytes;
  if (content.$case === "certificate") {
    bytes = content.certificate.rawBytes;
  } else if (content.$case === "x509CertificateChain") {
    bytes = content.x509CertificateChain.certificates[0]?.rawBytes;
  }
  if (!bytes || bytes.length === 0) {
    reject("provenance bundle has no signing certificate");
  }
  try {
    return new X509Certificate(Buffer.from(bytes));
  } catch (error) {
    reject("provenance signing certificate is invalid", undefined, {
      cause: error,
    });
  }
}

function certificateSeconds(certificate, property, fallback) {
  const value = certificate[property] ?? new Date(certificate[fallback]);
  const milliseconds = value instanceof Date ? value.valueOf() : Number.NaN;
  if (!Number.isFinite(milliseconds)) {
    reject("provenance certificate validity is invalid");
  }
  return Math.floor(milliseconds / 1000);
}

export function verifyEvidenceCryptographically(
  evidence,
  trustedRoot,
  registryKeys,
  expectedIssuer,
  expectedSubject,
  challenge,
  now,
) {
  const publishKey = registryKeys.get(evidence.publish.signature.keyid);
  if (!publishKey) {
    reject("publish attestation has no matching pinned npm registry key");
  }
  const keyFinder = (hint) => {
    const key = registryKeys.get(hint);
    if (!key) {
      reject("Sigstore requested an unpinned npm registry key");
    }
    return {
      publicKey: key.publicKey,
      validFor: (date) => key.expires === null || date < key.expires,
    };
  };
  let trustMaterial;
  try {
    trustMaterial = sigstoreVerify.toTrustMaterial(trustedRoot, keyFinder);
  } catch (error) {
    reject("pinned Sigstore trust material cannot be constructed", undefined, {
      cause: error,
    });
  }
  const verifier = new sigstoreVerify.Verifier(trustMaterial, {
    ctlogThreshold: 1,
    // @sigstore/verify treats each nonzero Rekor integratedTime as a
    // transparency-log timestamp. npm's current bundles have that verified
    // Rekor time but no RFC 3161 TSA token, so this requires one authenticated
    // signing time without requiring a separate TSA.
    timestampThreshold: 1,
    tlogThreshold: 1,
  });
  try {
    verifier.verify(sigstoreVerify.toSignedEntity(evidence.publish.parsedBundle));
  } catch (error) {
    reject("npm publish attestation cryptographic verification failed", undefined, {
      cause: error,
    });
  }
  try {
    verifier.verify(
      sigstoreVerify.toSignedEntity(evidence.provenance.parsedBundle),
      {
        extensions: { issuer: expectedIssuer },
        subjectAlternativeName: expectedSubject,
      },
    );
  } catch (error) {
    reject("npm provenance cryptographic/identity verification failed", undefined, {
      cause: error,
    });
  }

  const certificate = certificateFromBundle(evidence.provenance.parsedBundle);
  const issuedAt = certificateSeconds(
    certificate,
    "validFromDate",
    "validFrom",
  );
  const expiresAt = certificateSeconds(
    certificate,
    "validToDate",
    "validTo",
  );
  const integratedTimes = [
    evidence.publish.integratedTime,
    evidence.provenance.integratedTime,
  ];
  if (
    issuedAt < challenge.issued_at - MAX_CLOCK_SKEW_SECONDS ||
    issuedAt >= challenge.expires_at ||
    expiresAt <= issuedAt ||
    expiresAt < challenge.expires_at ||
    now < issuedAt - MAX_CLOCK_SKEW_SECONDS ||
    now >= expiresAt ||
    now >= challenge.expires_at ||
    integratedTimes.some(
      (time) =>
        time < challenge.issued_at - MAX_CLOCK_SKEW_SECONDS ||
        time >= challenge.expires_at,
    )
  ) {
    reject("certificate, transparency, challenge, and local times do not overlap safely");
  }
  return { proofIssuedAt: issuedAt, proofExpiresAt: expiresAt };
}

function selectPolicyEntry(policy, request, challenge) {
  const matches = policy.publishers.filter(
    (entry) =>
      entry.namespace === challenge.namespace &&
      entry.issuer === request.expected_issuer &&
      entry.subject === request.expected_subject,
  );
  if (matches.length !== 1) {
    reject("request publisher identity is not uniquely authorized by policy");
  }
  return matches[0];
}

export async function preverifyInvocation(
  raw,
  executableDirectory,
  { now = Math.floor(Date.now() / 1000) } = {},
) {
  requireSafeInteger(now, "verification time", 0);
  const validated = validateInvocation(raw, now);
  const policyRaw = loadPinnedFile(
    executableDirectory,
    POLICY_FILENAME,
    validated.invocation.policy_sha256,
    MAX_POLICY_BYTES,
    "npm authority policy",
  );
  const policy = validatePolicy(policyRaw, validated.invocation);
  const trustedRootRaw = loadPinnedFile(
    executableDirectory,
    TRUST_ROOT_FILENAME,
    validated.invocation.trust_root_sha256,
    MAX_TRUST_ROOT_BYTES,
    "Sigstore trusted root",
  );
  const registryKeysRaw = loadPinnedFile(
    executableDirectory,
    REGISTRY_KEYS_FILENAME,
    policy.registry_keys_sha256,
    MAX_REGISTRY_KEYS_BYTES,
    "npm registry keys",
  );
  const policyEntry = selectPolicyEntry(
    policy,
    validated.request,
    validated.challenge,
  );
  const packageIdentity = parsePackageIdentity(
    validated.request,
    validated.challenge,
  );
  inspectNpmTarball(
    validated.artifact,
    validated.challenge.challenge_document,
    policy.archive,
    validated.challenge,
    packageIdentity.packageName,
    packageIdentity.packageVersion,
  );
  const evidence = validateEvidence(
    validated.evidence,
    {
      ...packageIdentity,
      artifactDigest: validated.request.artifact_digest,
    },
    policyEntry,
    policy,
  );
  const trustedRoot = validateTrustedRoot(trustedRootRaw);
  const registryKeys = validateRegistryKeys(registryKeysRaw);
  const proofTime = verifyEvidenceCryptographically(
    evidence,
    trustedRoot,
    registryKeys,
    validated.request.expected_issuer,
    validated.request.expected_subject,
    validated.challenge,
    now,
  );
  return {
    ...validated,
    policy,
    policyEntry,
    packageIdentity,
    evidence,
    ...proofTime,
  };
}

export function enforceTrustedPublishingAuthenticationMode() {
  reject(
    "npm publish/provenance attestations do not cryptographically identify whether npm authentication used Trusted Publishing OIDC or a traditional token; npm_trusted_publisher authority remains unavailable",
    AUTH_MODE_ERROR,
  );
}

export async function verifyInvocation(raw, executableDirectory, options) {
  await preverifyInvocation(raw, executableDirectory, options);
  enforceTrustedPublishingAuthenticationMode();
}

export async function readBoundedStdin(stream = process.stdin) {
  const chunks = [];
  let total = 0;
  for await (const chunk of stream) {
    const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    total += bytes.length;
    if (total > MAX_STDIN_BYTES) {
      reject("stdin exceeds the authority-adapter protocol limit");
    }
    chunks.push(bytes);
  }
  if (total === 0) {
    reject("stdin is empty");
  }
  return Buffer.concat(chunks, total);
}

export async function main(executableDirectory) {
  try {
    if (
      process.env.CREG_AUTHORITY_ADAPTER_PROTOCOL !==
      String(PROTOCOL_VERSION)
    ) {
      reject("unsupported or missing authority-adapter protocol");
    }
    const raw = await readBoundedStdin();
    await verifyInvocation(raw, executableDirectory);
    reject("npm verifier reached an impossible post-gate state");
  } catch (error) {
    if (error instanceof Rejected) {
      process.stderr.write(`npm authority verification rejected: ${error.message}\n`);
    } else {
      process.stderr.write(
        "npm authority verification rejected: internal fail-closed error\n",
      );
    }
    process.exitCode = 1;
  }
}
