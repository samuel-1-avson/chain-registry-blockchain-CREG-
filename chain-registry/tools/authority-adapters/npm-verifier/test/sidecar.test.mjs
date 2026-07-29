import assert from "node:assert/strict";
import {
  createHash,
  createPrivateKey,
  createPublicKey,
  sign,
} from "node:crypto";
import { readFileSync } from "node:fs";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

import * as sigstoreVerify from "@sigstore/verify";

import { inspectNpmTarball } from "../src/archive.mjs";
import { Rejected } from "../src/errors.mjs";
import { canonicalize, parseJsonStrict } from "../src/strict-json.mjs";
import {
  AUTH_MODE_ERROR,
  PUBLISH_PREDICATE,
  PUBLISH_STATEMENT,
  enforceTrustedPublishingAuthenticationMode,
  parsePackageIdentity,
  preverifyInvocation,
  validateChallenge,
  validateInvocation,
  validatePolicy,
  validateStatementSubject,
} from "../src/sidecar.mjs";

const NOW = 1_900_000_000;
const PACKAGE = "demo";
const VERSION = "1.0.0";
const ISSUER = "https://token.actions.githubusercontent.com";
const SUBJECT =
  "https://github.com/example/demo/.github/workflows/release.yml@refs/tags/v1.0.0";
const root = fileURLToPath(new URL("../", import.meta.url));
const executable = fileURLToPath(
  new URL("../bin/creg-npm-authority-verifier.mjs", import.meta.url),
);

function sha(algorithm, bytes) {
  return createHash(algorithm).update(bytes).digest("hex");
}

function challenge(namespace = "npm:demo@") {
  const seed = Buffer.alloc(32, 7);
  const privateKey = createPrivateKey({
    key: Buffer.concat([
      Buffer.from("302e020100300506032b657004220420", "hex"),
      seed,
    ]),
    format: "der",
    type: "pkcs8",
  });
  const publicDer = createPublicKey(privateKey).export({
    format: "der",
    type: "spki",
  });
  const value = {
    schema_version: 1,
    purpose: "claim",
    challenge_document: "creg-claim.json",
    chain_id: "creg-testnet-1",
    genesis_hash: "44".repeat(32),
    namespace,
    current_controller_ed25519_pubkey: "",
    current_controller_evm_address: "",
    next_controller_ed25519_pubkey: publicDer.subarray(-32).toString("hex"),
    next_controller_evm_address: `0x${"88".repeat(20)}`,
    nonce: 7,
    issued_at: NOW - 10,
    expires_at: NOW + 300,
    next_controller_signature: "",
  };
  const intent = Buffer.from(
    [
      "creg-namespace-authority-intent-v1",
      "service:creg-package-ownership",
      "purpose:claim",
      "document:creg-claim.json",
      "chain_id:creg-testnet-1",
      `genesis_hash:${"44".repeat(32)}`,
      `namespace:${namespace}`,
      "current_ed25519_pubkey:",
      "current_evm_address:",
      `next_ed25519_pubkey:${value.next_controller_ed25519_pubkey}`,
      `next_evm_address:${value.next_controller_evm_address}`,
      "nonce:7",
      `issued_at:${NOW - 10}`,
      `expires_at:${NOW + 300}`,
    ].join("\n"),
  );
  value.next_controller_signature = sign(null, intent, privateKey).toString("hex");
  return value;
}

function writeText(header, offset, length, value) {
  const bytes = Buffer.from(value, "utf8");
  assert.ok(bytes.length <= length);
  bytes.copy(header, offset);
}

function writeOctal(header, offset, length, value) {
  const text = value.toString(8).padStart(length - 1, "0");
  assert.equal(text.length, length - 1);
  header.write(text, offset, length - 1, "ascii");
  header[offset + length - 1] = 0;
}

function tarHeader({ name, body, type = "0", link = "" }) {
  const header = Buffer.alloc(512);
  writeText(header, 0, 100, name);
  writeOctal(header, 100, 8, type === "5" ? 0o755 : 0o644);
  writeOctal(header, 108, 8, 0);
  writeOctal(header, 116, 8, 0);
  writeOctal(header, 124, 12, body.length);
  writeOctal(header, 136, 12, NOW);
  header.fill(0x20, 148, 156);
  header[156] = type.charCodeAt(0);
  writeText(header, 157, 100, link);
  Buffer.from([0x75, 0x73, 0x74, 0x61, 0x72, 0]).copy(header, 257);
  writeText(header, 263, 2, "00");
  writeText(header, 265, 32, "root");
  writeText(header, 297, 32, "root");
  let checksum = 0;
  for (const byte of header) {
    checksum += byte;
  }
  const encoded = checksum.toString(8).padStart(6, "0");
  header.write(encoded, 148, 6, "ascii");
  header[154] = 0;
  header[155] = 0x20;
  return header;
}

function rawTar(entries, trailing = Buffer.alloc(0)) {
  const chunks = [];
  for (const entry of entries) {
    const body = Buffer.from(entry.body ?? "");
    chunks.push(tarHeader({ ...entry, body }));
    chunks.push(body);
    const padding = (512 - (body.length % 512)) % 512;
    chunks.push(Buffer.alloc(padding));
  }
  chunks.push(Buffer.alloc(1024));
  chunks.push(trailing);
  return Buffer.concat(chunks);
}

function tarball(entries, trailing) {
  return gzipSync(rawTar(entries, trailing), { level: 9, mtime: 0 });
}

function archivePolicy(overrides = {}) {
  return {
    challenge_prefix: "package",
    package_manifest_path: "package/package.json",
    max_entries: 100,
    max_entry_bytes: 1024 * 1024,
    max_uncompressed_bytes: 8 * 1024 * 1024,
    max_compression_ratio: 200,
    ...overrides,
  };
}

function validTar(challengeBytes = canonicalize(challenge())) {
  return tarball([
    {
      name: "package/package.json",
      body: JSON.stringify({ name: PACKAGE, version: VERSION }),
    },
    { name: "package/creg-claim.json", body: challengeBytes },
    { name: "package/index.js", body: "export {};\n" },
  ]);
}

function invocation(artifact, evidence = Buffer.from("{}")) {
  return {
    schema_version: 2,
    adapter_id: "creg-npm-sigstore",
    adapter_version: "1.0.0",
    policy_sha256: "11".repeat(32),
    trust_root_sha256: "22".repeat(32),
    request: {
      schema_version: 2,
      authority: "npm_trusted_publisher",
      evidence_url:
        "https://registry.npmjs.org/-/npm/v1/attestations/demo@1.0.0",
      evidence_sha256: sha("sha256", evidence),
      evidence_hex: evidence.toString("hex"),
      artifact_url:
        "https://registry.npmjs.org/demo/-/demo-1.0.0.tgz",
      artifact_digest_algorithm: "sha512",
      artifact_digest: sha("sha512", artifact),
      artifact_hex: artifact.toString("hex"),
      expected_issuer: ISSUER,
      expected_subject: SUBJECT,
      expected_audience: "npm:registry.npmjs.org",
      challenge: challenge(),
    },
  };
}

function policy() {
  return {
    schema_version: 1,
    authority: "npm_trusted_publisher",
    adapter_id: "creg-npm-sigstore",
    adapter_version: "1.0.0",
    registry_origin: "https://registry.npmjs.org",
    registry_keys_sha256: "33".repeat(32),
    sigstore: {
      instance: "production",
      bundle_media_types: [
        "application/vnd.dev.sigstore.bundle+json;version=0.2",
        "application/vnd.dev.sigstore.bundle.v0.3+json",
      ],
    },
    publish: {
      statement_type: PUBLISH_STATEMENT,
      predicate_type: PUBLISH_PREDICATE,
    },
    authentication_mode: {
      required: "trusted_publishing_oidc",
      status: "unavailable",
      official_cryptographic_signal: null,
    },
    publishers: [
      {
        namespace: "npm:demo@",
        issuer: ISSUER,
        subject: SUBJECT,
        provenance_statement_type: "https://in-toto.io/Statement/v1",
        provenance_predicate_type: "https://slsa.dev/provenance/v1",
      },
    ],
    archive: archivePolicy(),
  };
}

test("strict JSON rejects duplicate members and invalid UTF-8", () => {
  assert.throws(
    () => parseJsonStrict(Buffer.from('{"x":1,"x":1}'), "fixture"),
    /duplicate object member/,
  );
  assert.throws(
    () => parseJsonStrict(Buffer.from([0xff]), "fixture"),
    /UTF-8/,
  );
});

test("RFC 8785-equivalent encoding is deterministic", () => {
  const value = parseJsonStrict(
    Buffer.from(' { "z": -0, "a": [3, true, "x"] } '),
    "fixture",
  );
  assert.equal(canonicalize(value).toString(), '{"a":[3,true,"x"],"z":0}');
});

test("controller challenge is bound to the Rust intent format", () => {
  const result = validateChallenge(challenge(), NOW);
  assert.equal(result.challenge.namespace, "npm:demo@");
  assert.match(result.authorityIntentSha256, /^[0-9a-f]{64}$/);
});

test("tampering a signed challenge fails closed", () => {
  const value = challenge();
  value.next_controller_evm_address = `0x${"99".repeat(20)}`;
  assert.throws(
    () => validateChallenge(value, NOW),
    /signature verification failed/,
  );
});

test("non-I-JSON-safe challenge integers are rejected", () => {
  const value = challenge();
  value.nonce = Number.MAX_SAFE_INTEGER + 1;
  assert.throws(
    () => validateChallenge(value, NOW),
    /integer range/,
  );
});

test("v2 invocation recomputes exact evidence SHA-256 and npm SHA-512", () => {
  const artifact = validTar();
  const value = invocation(artifact);
  const parsed = validateInvocation(canonicalize(value), NOW);
  assert.equal(parsed.artifact.length, artifact.length);
  value.request.artifact_digest = "00".repeat(64);
  assert.throws(
    () => validateInvocation(canonicalize(value), NOW),
    /npm SHA-512/,
  );
});

test("canonical scoped evidence, tarball, namespace, and PURL bind", () => {
  const identity = parsePackageIdentity(
    {
      evidence_url:
        "https://registry.npmjs.org/-/npm/v1/attestations/@scope%2fpkg@2.3.4",
      artifact_url:
        "https://registry.npmjs.org/@scope/pkg/-/pkg-2.3.4.tgz",
    },
    { namespace: "npm:@scope/pkg@" },
  );
  assert.deepEqual(identity, {
    packageName: "@scope/pkg",
    packageVersion: "2.3.4",
    purl: "pkg:npm/%40scope/pkg@2.3.4",
  });
});

test("alternate URL encoding cannot alias an npm package", () => {
  assert.throws(
    () =>
      parsePackageIdentity(
        {
          evidence_url:
            "https://registry.npmjs.org/-/npm/v1/attestations/%40scope%2Fpkg@2.3.4",
          artifact_url:
            "https://registry.npmjs.org/@scope/pkg/-/pkg-2.3.4.tgz",
        },
        { namespace: "npm:@scope/pkg@" },
      ),
    /canonical/,
  );
});

test("signed statement subject binds the exact PURL and SHA-512", () => {
  const artifactDigest = "ab".repeat(64);
  const statement = {
    _type: PUBLISH_STATEMENT,
    subject: [
      {
        name: "pkg:npm/demo@1.0.0",
        digest: { sha512: artifactDigest },
      },
    ],
    predicateType: PUBLISH_PREDICATE,
    predicate: {
      name: "demo",
      version: "1.0.0",
      registry: "https://registry.npmjs.org",
    },
  };
  validateStatementSubject(
    statement,
    "publish",
    { purl: "pkg:npm/demo@1.0.0", artifactDigest },
    PUBLISH_STATEMENT,
    PUBLISH_PREDICATE,
  );
  statement.subject[0].name = "pkg:npm/other@1.0.0";
  assert.throws(
    () =>
      validateStatementSubject(
        statement,
        "publish",
        { purl: "pkg:npm/demo@1.0.0", artifactDigest },
        PUBLISH_STATEMENT,
        PUBLISH_PREDICATE,
      ),
    /exact PURL/,
  );
});

test("archive reads one RFC 8785-equivalent root challenge in memory", () => {
  const expected = challenge();
  const pretty = Buffer.from(JSON.stringify(expected, null, 2));
  const artifact = validTar(pretty);
  const inspected = inspectNpmTarball(
    artifact,
    "creg-claim.json",
    archivePolicy(),
    expected,
    PACKAGE,
    VERSION,
  );
  assert.ok(inspected.challengeBytes.equals(pretty));
});

test("archive rejects a second CREG challenge anywhere", () => {
  const expected = challenge();
  const artifact = tarball([
    {
      name: "package/package.json",
      body: JSON.stringify({ name: PACKAGE, version: VERSION }),
    },
    { name: "package/creg-claim.json", body: canonicalize(expected) },
    { name: "package/nested/creg-recovery.json", body: "{}" },
  ]);
  assert.throws(
    () =>
      inspectNpmTarball(
        artifact,
        "creg-claim.json",
        archivePolicy(),
        expected,
        PACKAGE,
        VERSION,
      ),
    /exactly one CREG challenge/,
  );
});

test("archive rejects traversing and case-colliding paths", () => {
  const expected = challenge();
  const traversal = tarball([
    {
      name: "package/package.json",
      body: JSON.stringify({ name: PACKAGE, version: VERSION }),
    },
    { name: "package/creg-claim.json", body: canonicalize(expected) },
    { name: "../escape", body: "x" },
  ]);
  assert.throws(
    () =>
      inspectNpmTarball(
        traversal,
        "creg-claim.json",
        archivePolicy(),
        expected,
        PACKAGE,
        VERSION,
      ),
    /traversing/,
  );

  const collision = tarball([
    {
      name: "package/package.json",
      body: JSON.stringify({ name: PACKAGE, version: VERSION }),
    },
    { name: "package/creg-claim.json", body: canonicalize(expected) },
    { name: "package/File.js", body: "" },
    { name: "package/file.js", body: "" },
  ]);
  assert.throws(
    () =>
      inspectNpmTarball(
        collision,
        "creg-claim.json",
        archivePolicy(),
        expected,
        PACKAGE,
        VERSION,
      ),
    /case-colliding/,
  );
});

test("archive rejects links, PAX metadata, and trailing archives", () => {
  const expected = challenge();
  for (const entry of [
    { name: "package/link", type: "2", link: "package/creg-claim.json" },
    { name: "PaxHeader", type: "x", body: "12 path=x\n" },
  ]) {
    const artifact = tarball([
      {
        name: "package/package.json",
        body: JSON.stringify({ name: PACKAGE, version: VERSION }),
      },
      { name: "package/creg-claim.json", body: canonicalize(expected) },
      entry,
    ]);
    assert.throws(
      () =>
        inspectNpmTarball(
          artifact,
          "creg-claim.json",
          archivePolicy(),
          expected,
          PACKAGE,
          VERSION,
        ),
      /link|extended|special/,
    );
  }

  const trailing = tarball(
    [
      {
        name: "package/package.json",
        body: JSON.stringify({ name: PACKAGE, version: VERSION }),
      },
      { name: "package/creg-claim.json", body: canonicalize(expected) },
    ],
    Buffer.alloc(512, 1),
  );
  assert.throws(
    () =>
      inspectNpmTarball(
        trailing,
        "creg-claim.json",
        archivePolicy(),
        expected,
        PACKAGE,
        VERSION,
      ),
    /trailing data/,
  );
});

test("archive rejects compression bombs before parsing entries", () => {
  const artifact = validTar();
  assert.throws(
    () =>
      inspectNpmTarball(
        artifact,
        "creg-claim.json",
        archivePolicy({ max_compression_ratio: 1 }),
        challenge(),
        PACKAGE,
        VERSION,
      ),
    /compression-ratio/,
  );
});

test("archive package.json must bind exact name and version", () => {
  const expected = challenge();
  const artifact = tarball([
    {
      name: "package/package.json",
      body: JSON.stringify({ name: "other", version: VERSION }),
    },
    { name: "package/creg-claim.json", body: canonicalize(expected) },
  ]);
  assert.throws(
    () =>
      inspectNpmTarball(
        artifact,
        "creg-claim.json",
        archivePolicy(),
        expected,
        PACKAGE,
        VERSION,
      ),
    /exact package and version/,
  );
});

test("policy is canonical, version-bound, and cannot enable auth mode", () => {
  const invocationValue = invocation(validTar());
  const value = policy();
  const parsed = validatePolicy(canonicalize(value), invocationValue);
  assert.equal(parsed.authentication_mode.status, "unavailable");

  const pretty = Buffer.from(JSON.stringify(value, null, 2));
  assert.throws(
    () => validatePolicy(pretty, invocationValue),
    /not canonical RFC 8785/,
  );
  value.authentication_mode.status = "verified";
  value.authentication_mode.official_cryptographic_signal = "provenance";
  assert.throws(
    () => validatePolicy(canonicalize(value), invocationValue),
    /bypass/,
  );
});

test("authentication-mode gate is unconditional and specifically classified", () => {
  assert.throws(
    () => enforceTrustedPublishingAuthenticationMode(),
    (error) =>
      error instanceof Rejected &&
      error.code === AUTH_MODE_ERROR &&
      /traditional token/.test(error.message),
  );
});

test("Sigstore treats npm's Rekor integrated time as a verifier timestamp", () => {
  const tlogEntry = { integratedTime: "1900000000" };
  const entity = sigstoreVerify.toSignedEntity({
    content: {
      $case: "dsseEnvelope",
      dsseEnvelope: {
        payload: Buffer.alloc(0),
        payloadType: "application/vnd.in-toto+json",
        signatures: [],
      },
    },
    verificationMaterial: {
      content: {
        $case: "publicKey",
        publicKey: { hint: "test" },
      },
      timestampVerificationData: { rfc3161Timestamps: [] },
      tlogEntries: [tlogEntry],
    },
  });
  assert.deepEqual(
    entity.timestamps.map((timestamp) => timestamp.$case),
    ["transparency-log"],
  );
  assert.equal(entity.timestamps[0].tlogEntry, tlogEntry);
});

test("CLI never writes a verified result or any stdout on rejection", () => {
  const child = spawnSync(process.execPath, [executable], {
    cwd: root,
    input: "{}",
    encoding: "utf8",
    env: { CREG_AUTHORITY_ADAPTER_PROTOCOL: "2" },
  });
  assert.notEqual(child.status, 0);
  assert.equal(child.stdout, "");
  assert.match(child.stderr, /npm authority verification rejected/);
});

test(
  "fresh official production fixture reaches preverification then still gates",
  {
    skip: !process.env.CREG_NPM_PRODUCTION_FIXTURE_DIR,
  },
  async () => {
    const directory = process.env.CREG_NPM_PRODUCTION_FIXTURE_DIR;
    const raw = await readFile(
      join(directory, "authority-adapter-invocation-v2.jcs.json"),
    );
    const preverified = await preverifyInvocation(raw, directory);
    assert.equal(preverified.request.authority, "npm_trusted_publisher");
    assert.throws(
      () => enforceTrustedPublishingAuthenticationMode(),
      (error) => error instanceof Rejected && error.code === AUTH_MODE_ERROR,
    );
  },
);

test("policy schema documents the immutable unavailable signal", () => {
  const schema = JSON.parse(
    readFileSync(new URL("../npm-policy.schema.json", import.meta.url), "utf8"),
  );
  assert.equal(
    schema.properties.authentication_mode.properties.status.const,
    "unavailable",
  );
  assert.equal(
    schema.properties.authentication_mode.properties
      .official_cryptographic_signal.type,
    "null",
  );
});
