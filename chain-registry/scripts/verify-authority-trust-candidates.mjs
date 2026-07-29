#!/usr/bin/env node

import {
  createHash,
  createPublicKey,
} from "node:crypto";
import {
  readFileSync,
} from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  canonicalize,
  parseJsonStrict,
} from "../tools/authority-adapters/npm-verifier/src/strict-json.mjs";

const SCRIPT_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_CANDIDATE_DIR =
  "tools/authority-adapters/trust-candidates";
const ROOT_FILENAME = "sigstore-production-trusted-root.json";
const NPM_KEYS_FILENAME = "npm-registry-keys.json";
const MANIFEST_FILENAME = "candidate-manifest.json";
const SIGSTORE_SOURCE =
  "https://tuf-repo-cdn.sigstore.dev/targets/trusted_root.json";
const NPM_KEYS_SOURCE = "https://registry.npmjs.org/-/npm/v1/keys";
const SIGSTORE_MEDIA_TYPE =
  "application/vnd.dev.sigstore.trustedroot+json;version=0.1";
const REQUIRED_BLOCKERS = new Set([
  "independent trust-root and npm-key review has not been recorded",
  "namespace-specific canonical publisher policies have not been approved",
  "npm exposes no signed signal proving Trusted Publishing authentication mode",
  "no signed CR-07 activation manifest or operator rehearsal exists",
]);

function fail(message) {
  throw new Error(`authority trust-candidate gate: ${message}`);
}

function sha256(data) {
  return createHash("sha256").update(data).digest("hex");
}

function exactObject(value, label, keys) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${label} must be an object`);
  }
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (
    actual.length !== expected.length ||
    actual.some((key, index) => key !== expected[index])
  ) {
    fail(`${label} has unexpected fields`);
  }
  return value;
}

function boundedFile(path, label, maximum) {
  let raw;
  try {
    raw = readFileSync(path);
  } catch (error) {
    fail(`cannot read ${label}: ${error.message}`);
  }
  if (raw.length === 0 || raw.length > maximum) {
    fail(`${label} has invalid size`);
  }
  return raw;
}

function verifyPinnedFile(descriptor, expected, directory, maximum, label) {
  exactObject(descriptor, label, [
    "source",
    "verification",
    "file",
    "bytes",
    "sha256",
  ]);
  if (
    descriptor.source !== expected.source ||
    descriptor.verification !== expected.verification ||
    descriptor.file !== expected.file ||
    !Number.isSafeInteger(descriptor.bytes) ||
    descriptor.bytes <= 0 ||
    !/^[0-9a-f]{64}$/u.test(descriptor.sha256)
  ) {
    fail(`${label} metadata is not canonical`);
  }
  const raw = boundedFile(resolve(directory, expected.file), label, maximum);
  if (raw.length !== descriptor.bytes || sha256(raw) !== descriptor.sha256) {
    fail(`${label} bytes do not match the manifest pin`);
  }
  return raw;
}

function parseRetrievalTime(value) {
  if (
    typeof value !== "string" ||
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/u.test(value)
  ) {
    fail("retrieved_at is not canonical UTC");
  }
  const parsed = new Date(value);
  if (Number.isNaN(parsed.valueOf())) {
    fail("retrieved_at is invalid");
  }
  return parsed;
}

function verifyTrustedRoot(raw) {
  const root = exactObject(
    parseJsonStrict(raw, "Sigstore TrustedRoot", {
      maxBytes: 2 * 1024 * 1024,
      maxDepth: 128,
    }),
    "Sigstore TrustedRoot",
    [
      "certificateAuthorities",
      "ctlogs",
      "mediaType",
      "tlogs",
      "timestampAuthorities",
    ],
  );
  if (
    root.mediaType !== SIGSTORE_MEDIA_TYPE ||
    !Array.isArray(root.certificateAuthorities) ||
    root.certificateAuthorities.length === 0 ||
    !Array.isArray(root.tlogs) ||
    root.tlogs.length === 0 ||
    !Array.isArray(root.ctlogs) ||
    root.ctlogs.length === 0
  ) {
    fail("Sigstore TrustedRoot lacks the production verification material");
  }
}

function verifyNpmKeys(raw, retrievalTime, declaredActiveKeys) {
  const material = exactObject(
    parseJsonStrict(raw, "npm registry keys", {
      maxBytes: 256 * 1024,
      maxDepth: 16,
    }),
    "npm registry keys",
    ["keys"],
  );
  if (
    !Array.isArray(material.keys) ||
    material.keys.length === 0 ||
    material.keys.length > 64
  ) {
    fail("npm registry key list is empty or excessive");
  }
  const seen = new Set();
  const active = [];
  for (const [index, item] of material.keys.entries()) {
    exactObject(item, `npm registry key ${index}`, [
      "expires",
      "key",
      "keyid",
      "keytype",
      "scheme",
    ]);
    if (
      typeof item.keyid !== "string" ||
      !item.keyid.startsWith("SHA256:") ||
      seen.has(item.keyid) ||
      item.keytype !== "ecdsa-sha2-nistp256" ||
      item.scheme !== "ecdsa-sha2-nistp256"
    ) {
      fail(`npm registry key ${index} metadata is unsupported or duplicate`);
    }
    seen.add(item.keyid);
    let publicKey;
    try {
      publicKey = createPublicKey({
        key: Buffer.from(item.key, "base64"),
        format: "der",
        type: "spki",
      });
    } catch (error) {
      fail(`npm registry key ${index} is not valid DER SPKI: ${error.message}`);
    }
    if (
      publicKey.asymmetricKeyType !== "ec" ||
      publicKey.asymmetricKeyDetails?.namedCurve !== "prime256v1"
    ) {
      fail(`npm registry key ${index} is not ECDSA P-256`);
    }
    if (item.expires === null) {
      active.push(item.keyid);
    } else if (typeof item.expires === "string") {
      const expiry = new Date(item.expires);
      if (Number.isNaN(expiry.valueOf())) {
        fail(`npm registry key ${index} has invalid expiry`);
      }
      if (expiry > retrievalTime) {
        active.push(item.keyid);
      }
    } else {
      fail(`npm registry key ${index} has invalid expiry`);
    }
  }
  if (
    !Array.isArray(declaredActiveKeys) ||
    JSON.stringify([...declaredActiveKeys].sort()) !==
      JSON.stringify(active.sort())
  ) {
    fail("manifest active npm key list does not match retrieval-time validity");
  }
}

function rejectUnapprovedActivation(root, trustedRootSha256) {
  const activationManifestPath = resolve(
    root,
    "config/cr07-activation-manifest.json",
  );
  let activationManifest;
  try {
    activationManifest = JSON.parse(
      readFileSync(activationManifestPath, "utf8"),
    );
  } catch (error) {
    fail(`cannot inspect CR-07 activation manifest: ${error.message}`);
  }
  if (!Array.isArray(activationManifest.guarded_chain_specs)) {
    fail("CR-07 activation manifest has no guarded chain-spec list");
  }
  for (const specPath of activationManifest.guarded_chain_specs) {
    let spec;
    try {
      spec = JSON.parse(readFileSync(resolve(root, specPath), "utf8"));
    } catch (error) {
      fail(`cannot inspect guarded chain spec ${specPath}: ${error.message}`);
    }
    const policies =
      spec?.consensus_params?.namespace_authority_policies ?? [];
    if (
      policies.some(
        (policy) => policy?.trust_root_sha256 === trustedRootSha256,
      )
    ) {
      fail(
        `${specPath} activates the repository's independently unapproved trust candidate`,
      );
    }
  }
}

export function verifyTrustCandidates({
  root = SCRIPT_ROOT,
  candidateDirectory = DEFAULT_CANDIDATE_DIR,
} = {}) {
  root = resolve(root);
  const directory = resolve(root, candidateDirectory);
  const manifestRaw = boundedFile(
    resolve(directory, MANIFEST_FILENAME),
    "candidate manifest",
    64 * 1024,
  );
  const manifest = exactObject(
    parseJsonStrict(manifestRaw, "candidate manifest", {
      maxBytes: 64 * 1024,
      maxDepth: 16,
    }),
    "candidate manifest",
    [
      "activation_blockers",
      "npm_registry_keys",
      "retrieved_at",
      "schema_version",
      "sigstore",
      "status",
    ],
  );
  if (
    !manifestRaw.equals(Buffer.concat([canonicalize(manifest), Buffer.from("\n")]))
  ) {
    fail("candidate manifest is not canonical JSON plus one newline");
  }
  if (manifest.schema_version !== 1 || manifest.status !== "candidate_unapproved") {
    fail("candidate manifest must remain version 1 and independently unapproved");
  }
  if (
    !Array.isArray(manifest.activation_blockers) ||
    manifest.activation_blockers.length !== REQUIRED_BLOCKERS.size ||
    manifest.activation_blockers.some((blocker) => !REQUIRED_BLOCKERS.has(blocker))
  ) {
    fail("candidate manifest does not retain every activation blocker");
  }
  const retrievalTime = parseRetrievalTime(manifest.retrieved_at);

  const trustedRoot = verifyPinnedFile(
    manifest.sigstore,
    {
      source: SIGSTORE_SOURCE,
      verification: "sigstore-python-4.5.0-tuf",
      file: ROOT_FILENAME,
    },
    directory,
    2 * 1024 * 1024,
    "Sigstore TrustedRoot",
  );
  verifyTrustedRoot(trustedRoot);

  exactObject(manifest.npm_registry_keys, "npm registry-key descriptor", [
    "active_key_ids_at_retrieval",
    "source",
    "verification",
    "file",
    "bytes",
    "sha256",
  ]);
  const npmDescriptor = {
    ...manifest.npm_registry_keys,
  };
  delete npmDescriptor.active_key_ids_at_retrieval;
  const npmKeys = verifyPinnedFile(
    npmDescriptor,
    {
      source: NPM_KEYS_SOURCE,
      verification: "official-https-endpoint-only",
      file: NPM_KEYS_FILENAME,
    },
    directory,
    256 * 1024,
    "npm registry keys",
  );
  verifyNpmKeys(
    npmKeys,
    retrievalTime,
    manifest.npm_registry_keys.active_key_ids_at_retrieval,
  );
  rejectUnapprovedActivation(root, manifest.sigstore.sha256);

  return {
    manifestSha256: sha256(manifestRaw),
    trustedRootSha256: manifest.sigstore.sha256,
    npmRegistryKeysSha256: manifest.npm_registry_keys.sha256,
  };
}

function usage() {
  console.log("Usage: node scripts/verify-authority-trust-candidates.mjs");
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 2) {
    usage();
    process.exitCode = 1;
  } else {
    try {
      const result = verifyTrustCandidates();
      console.log(JSON.stringify({ status: "valid_unapproved_candidate", ...result }));
    } catch (error) {
      console.error(error.message);
      process.exitCode = 1;
    }
  }
}
