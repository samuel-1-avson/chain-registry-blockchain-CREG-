#!/usr/bin/env node

// This is deliberately NOT a verifier. It exists so process-boundary tests can
// exercise a sidecar that can never accidentally issue a successful authority
// observation while production cryptographic verification is unavailable.

const PROTOCOL_VERSION = "2";
// Protocol v2 permits 64 MiB of artifact bytes and 8 MiB of evidence bytes.
// Both are hex encoded; reserve another MiB for the invocation envelope.
const MAX_STDIN_BYTES = 145 * 1024 * 1024;

function reject(message, exitCode) {
  process.stderr.write(`CREG authority adapter unavailable: ${message}\n`);
  process.exitCode = exitCode;
}

if (process.env.CREG_AUTHORITY_ADAPTER_PROTOCOL !== PROTOCOL_VERSION) {
  reject("unsupported or missing protocol version", 64);
} else {
  const chunks = [];
  let size = 0;
  let oversized = false;

  for await (const chunk of process.stdin) {
    size += chunk.length;
    if (size > MAX_STDIN_BYTES) {
      oversized = true;
      break;
    }
    chunks.push(chunk);
  }

  if (oversized) {
    reject("request exceeds the v2 hard limit", 65);
  } else {
    try {
      const request = JSON.parse(Buffer.concat(chunks).toString("utf8"));
      if (
        request === null ||
        Array.isArray(request) ||
        typeof request !== "object" ||
        request.schema_version !== 2
      ) {
        reject("invalid v2 request framing", 65);
      } else {
        reject(
          "production cryptographic verifier and pinned trust policy are not installed",
          78,
        );
      }
    } catch {
      reject("request is not valid JSON", 65);
    }
  }
}
