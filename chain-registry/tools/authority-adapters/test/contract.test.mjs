import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const stub = fileURLToPath(
  new URL("../src/rejection-stub.mjs", import.meta.url),
);

test("request and result schemas are strict JSON objects", () => {
  for (const name of [
    "authority-adapter-request-v2.schema.json",
    "authority-adapter-result-v2.schema.json",
  ]) {
    const schema = JSON.parse(
      readFileSync(new URL(`../schemas/${name}`, import.meta.url), "utf8"),
    );
    assert.equal(schema.type, "object");
    assert.equal(schema.additionalProperties, false);
    assert.equal(schema.properties.schema_version.const, 2);
    assert.ok(schema.required.length > 0);
  }
});

test("v2 schemas bind the exact artifact identity on both sides", () => {
  const request = JSON.parse(
    readFileSync(
      new URL(
        "../schemas/authority-adapter-request-v2.schema.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const result = JSON.parse(
    readFileSync(
      new URL(
        "../schemas/authority-adapter-result-v2.schema.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );

  for (const field of [
    "artifact_url",
    "artifact_digest_algorithm",
    "artifact_digest",
    "artifact_hex",
  ]) {
    assert.ok(request.required.includes(field), `${field} must be required`);
  }
  for (const field of [
    "artifact_url",
    "artifact_digest_algorithm",
    "artifact_digest",
  ]) {
    assert.ok(result.required.includes(field), `${field} must be echoed`);
  }
  assert.equal(request.properties.artifact_hex.maxLength, 134217728);
});

test("verified_at is required but explicitly validator-local", () => {
  const result = JSON.parse(
    readFileSync(
      new URL(
        "../schemas/authority-adapter-result-v2.schema.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const readme = readFileSync(new URL("../README.md", import.meta.url), "utf8");

  assert.ok(result.required.includes("verified_at"));
  assert.match(result.properties.verified_at.description, /validator-local/i);
  assert.match(result.properties.verified_at.description, /MUST NOT/);
  assert.match(result.properties.verified_at.description, /must still aggregate/i);
  assert.match(readme, /operator\s+ACL/);
  assert.match(readme, /hard maximum of \*\*4 concurrent/);
});

test("the repository fixture sidecar can never emit a verified result", () => {
  const request = JSON.stringify({
    schema_version: 2,
    authority: "pypi_trusted_publisher",
    artifact_url:
      "https://files.pythonhosted.org/packages/demo-1.0-py3-none-any.whl",
    artifact_digest_algorithm: "sha256",
    artifact_digest: "00".repeat(32),
    artifact_hex: "00",
  });
  const child = spawnSync(process.execPath, [stub], {
    cwd: root,
    input: request,
    encoding: "utf8",
    env: { CREG_AUTHORITY_ADAPTER_PROTOCOL: "2" },
  });

  assert.equal(child.status, 78);
  assert.equal(child.stdout, "");
  assert.match(child.stderr, /production cryptographic verifier/);
});

test("the rejection fixture also fails closed on protocol drift", () => {
  const child = spawnSync(process.execPath, [stub], {
    cwd: root,
    input: "{}",
    encoding: "utf8",
    env: { CREG_AUTHORITY_ADAPTER_PROTOCOL: "1" },
  });

  assert.equal(child.status, 64);
  assert.equal(child.stdout, "");
  assert.match(child.stderr, /unsupported or missing protocol version/);
});
