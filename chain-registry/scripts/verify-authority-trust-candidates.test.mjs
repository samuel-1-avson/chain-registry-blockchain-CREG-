import assert from "node:assert/strict";
import {
  cpSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import test from "node:test";
import { canonicalize } from "../tools/authority-adapters/npm-verifier/src/strict-json.mjs";
import { verifyTrustCandidates } from "./verify-authority-trust-candidates.mjs";

const ROOT = resolve(import.meta.dirname, "..");

function fixture() {
  const temporary = mkdtempSync(resolve(tmpdir(), "creg-trust-candidate-test-"));
  cpSync(
    resolve(ROOT, "tools/authority-adapters/trust-candidates"),
    resolve(temporary, "tools/authority-adapters/trust-candidates"),
    { recursive: true },
  );
  cpSync(
    resolve(ROOT, "config/cr07-activation-manifest.json"),
    resolve(temporary, "config/cr07-activation-manifest.json"),
    { recursive: true },
  );
  const activation = JSON.parse(
    readFileSync(resolve(temporary, "config/cr07-activation-manifest.json")),
  );
  for (const path of activation.guarded_chain_specs) {
    cpSync(resolve(ROOT, path), resolve(temporary, path), { recursive: true });
  }
  return temporary;
}

function mutateManifest(root, mutate) {
  const path = resolve(
    root,
    "tools/authority-adapters/trust-candidates/candidate-manifest.json",
  );
  const manifest = JSON.parse(readFileSync(path, "utf8"));
  mutate(manifest);
  writeFileSync(path, Buffer.concat([canonicalize(manifest), Buffer.from("\n")]));
}

test("tracked official candidates match every unapproved pin", () => {
  const result = verifyTrustCandidates({ root: ROOT });
  assert.match(result.manifestSha256, /^[0-9a-f]{64}$/u);
  assert.match(result.trustedRootSha256, /^[0-9a-f]{64}$/u);
  assert.match(result.npmRegistryKeysSha256, /^[0-9a-f]{64}$/u);
});

test("tampered TrustedRoot bytes are rejected", () => {
  const root = fixture();
  try {
    const path = resolve(
      root,
      "tools/authority-adapters/trust-candidates/sigstore-production-trusted-root.json",
    );
    writeFileSync(path, Buffer.concat([readFileSync(path), Buffer.from(" ")]));
    assert.throws(() => verifyTrustCandidates({ root }), /manifest pin/u);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a duplicate manifest member is rejected before JSON interpretation", () => {
  const root = fixture();
  try {
    const path = resolve(
      root,
      "tools/authority-adapters/trust-candidates/candidate-manifest.json",
    );
    const raw = readFileSync(path, "utf8").trimEnd();
    writeFileSync(path, raw.replace(/^\{/u, '{"status":"candidate_unapproved",') + "\n");
    assert.throws(() => verifyTrustCandidates({ root }), /duplicate object member/u);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("retrieval-time active npm keys must match the official response", () => {
  const root = fixture();
  try {
    mutateManifest(root, (manifest) => {
      manifest.npm_registry_keys.active_key_ids_at_retrieval = [];
    });
    assert.throws(() => verifyTrustCandidates({ root }), /active npm key list/u);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("the repository candidate cannot be activated while unapproved", () => {
  const root = fixture();
  try {
    const activationPath = resolve(root, "config/cr07-activation-manifest.json");
    const activation = JSON.parse(readFileSync(activationPath, "utf8"));
    const specPath = resolve(root, activation.guarded_chain_specs[0]);
    const spec = JSON.parse(readFileSync(specPath, "utf8"));
    const candidate = JSON.parse(
      readFileSync(
        resolve(
          root,
          "tools/authority-adapters/trust-candidates/candidate-manifest.json",
        ),
        "utf8",
      ),
    );
    spec.consensus_params.namespace_authority_policies = [
      { trust_root_sha256: candidate.sigstore.sha256 },
    ];
    writeFileSync(specPath, JSON.stringify(spec));
    assert.throws(
      () => verifyTrustCandidates({ root }),
      /independently unapproved trust candidate/u,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
