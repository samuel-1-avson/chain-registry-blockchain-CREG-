import { gunzipSync } from "node:zlib";

import { parseJsonStrict, canonicalize } from "./strict-json.mjs";
import { reject } from "./errors.mjs";

export const MAX_CHALLENGE_BYTES = 64 * 1024;
const MAX_MANIFEST_BYTES = 1024 * 1024;
const BLOCK = 512;
const decoder = new TextDecoder("utf-8", { fatal: true });

function allZero(bytes) {
  return bytes.every((byte) => byte === 0);
}

function tarString(header, start, length, label) {
  const field = header.subarray(start, start + length);
  const zero = field.indexOf(0);
  const content = zero === -1 ? field : field.subarray(0, zero);
  if (zero !== -1 && !allZero(field.subarray(zero))) {
    reject(`TAR ${label} has non-zero bytes after its terminator`);
  }
  try {
    return decoder.decode(content);
  } catch (error) {
    reject(`TAR ${label} is not UTF-8`, undefined, { cause: error });
  }
}

function tarOctal(header, start, length, label) {
  const field = header.subarray(start, start + length);
  if (field[0] & 0x80) {
    reject(`TAR ${label} uses unsupported base-256 encoding`);
  }
  const text = Buffer.from(field)
    .toString("ascii")
    .replace(/\0.*$/s, "")
    .trim();
  if (text === "") {
    return 0;
  }
  if (!/^[0-7]+$/.test(text)) {
    reject(`TAR ${label} is not canonical octal`);
  }
  const value = Number.parseInt(text, 8);
  if (!Number.isSafeInteger(value) || value < 0) {
    reject(`TAR ${label} is outside the safe integer range`);
  }
  return value;
}

function validateChecksum(header) {
  const expected = tarOctal(header, 148, 8, "checksum");
  let actual = 0;
  for (let index = 0; index < BLOCK; index += 1) {
    actual += index >= 148 && index < 156 ? 0x20 : header[index];
  }
  if (actual !== expected) {
    reject("TAR header checksum is invalid");
  }
}

function validatePath(path, type) {
  if (
    path.length === 0 ||
    Buffer.byteLength(path, "utf8") > 4096 ||
    path !== path.normalize("NFC") ||
    path.includes("\\") ||
    path.startsWith("/") ||
    /^[A-Za-z]:/.test(path)
  ) {
    reject("TAR entry path is absolute, non-canonical, or invalid");
  }
  const parts = path.split("/");
  if (type === "directory" && parts.at(-1) === "") {
    parts.pop();
  }
  if (
    parts.length === 0 ||
    parts.some((part) => part === "" || part === "." || part === "..")
  ) {
    reject("TAR entry path is empty or traversing");
  }
  return parts;
}

function requireUstar(header) {
  const magic = header.subarray(257, 263);
  const version = header.subarray(263, 265);
  if (
    !magic.equals(Buffer.from([0x75, 0x73, 0x74, 0x61, 0x72, 0x00])) ||
    !version.equals(Buffer.from("00", "ascii"))
  ) {
    reject("archive is not a standard USTAR stream");
  }
}

function decodeTar(artifact, policy) {
  if (artifact.length < 2 || artifact[0] !== 0x1f || artifact[1] !== 0x8b) {
    reject("npm artifact is not a gzip stream");
  }
  let raw;
  try {
    raw = gunzipSync(artifact, {
      maxOutputLength: policy.max_uncompressed_bytes,
    });
  } catch (error) {
    reject("npm artifact gzip stream is invalid or exceeds policy", undefined, {
      cause: error,
    });
  }
  if (
    raw.length === 0 ||
    raw.length > policy.max_uncompressed_bytes ||
    raw.length > Math.max(artifact.length, 1) * policy.max_compression_ratio
  ) {
    reject("npm artifact exceeds uncompressed-size or compression-ratio policy");
  }
  if (raw.length % BLOCK !== 0) {
    reject("TAR stream is not block aligned");
  }
  return raw;
}

export function inspectNpmTarball(
  artifact,
  document,
  archivePolicy,
  expectedChallenge,
  packageName,
  packageVersion,
) {
  const raw = decodeTar(artifact, archivePolicy);
  const names = new Set();
  const foldedNames = new Set();
  const challengeEntries = [];
  let manifest;
  let entries = 0;
  let declaredBytes = 0;
  let offset = 0;
  let terminators = 0;

  while (offset + BLOCK <= raw.length) {
    const header = raw.subarray(offset, offset + BLOCK);
    if (allZero(header)) {
      terminators += 1;
      offset += BLOCK;
      if (terminators === 2) {
        break;
      }
      continue;
    }
    if (terminators !== 0) {
      reject("TAR contains an entry after a partial end marker");
    }
    entries += 1;
    if (entries > archivePolicy.max_entries) {
      reject("TAR entry count exceeds policy");
    }
    validateChecksum(header);
    requireUstar(header);
    const flag = header[156];
    const type =
      flag === 0 || flag === 0x30
        ? "file"
        : flag === 0x35
          ? "directory"
          : null;
    if (type === null) {
      reject("TAR contains a link, sparse, extended, or special entry");
    }
    if (!allZero(header.subarray(157, 257))) {
      reject("TAR entry unexpectedly contains a link target");
    }
    const name = tarString(header, 0, 100, "name");
    const prefix = tarString(header, 345, 155, "prefix");
    const path = prefix ? `${prefix}/${name}` : name;
    const parts = validatePath(path, type);
    const folded = path.normalize("NFC").toLowerCase();
    if (names.has(path) || foldedNames.has(folded)) {
      reject("TAR contains a duplicate or case-colliding entry path");
    }
    names.add(path);
    foldedNames.add(folded);

    const size = tarOctal(header, 124, 12, "entry size");
    if (type === "directory" && size !== 0) {
      reject("TAR directory has a non-zero size");
    }
    if (size > archivePolicy.max_entry_bytes) {
      reject("TAR entry exceeds the per-entry policy limit");
    }
    declaredBytes += size;
    if (declaredBytes > archivePolicy.max_uncompressed_bytes) {
      reject("TAR declared size exceeds policy");
    }
    const dataStart = offset + BLOCK;
    const padded = Math.ceil(size / BLOCK) * BLOCK;
    const next = dataStart + padded;
    if (next > raw.length || dataStart + size > next) {
      reject("TAR entry extends beyond the bounded stream");
    }

    if (
      parts.at(-1) === "creg-claim.json" ||
      parts.at(-1) === "creg-recovery.json"
    ) {
      if (type !== "file" || size > MAX_CHALLENGE_BYTES) {
        reject("challenge TAR entry is not a bounded regular file");
      }
      challengeEntries.push({
        path,
        bytes: raw.subarray(dataStart, dataStart + size),
      });
    }
    if (path === archivePolicy.package_manifest_path) {
      if (type !== "file" || size === 0 || size > MAX_MANIFEST_BYTES) {
        reject("npm package manifest is not a bounded regular file");
      }
      manifest = raw.subarray(dataStart, dataStart + size);
    }
    offset = next;
  }

  if (terminators !== 2 || entries === 0 || !allZero(raw.subarray(offset))) {
    reject("TAR stream has no canonical end marker or has trailing data");
  }
  if (challengeEntries.length !== 1) {
    reject("artifact must contain exactly one CREG challenge document");
  }
  const challengeEntry = challengeEntries[0];
  const expectedPath = `${archivePolicy.challenge_prefix}/${document}`;
  if (challengeEntry.path !== expectedPath) {
    reject("challenge document is not at the policy-pinned archive path");
  }
  if (manifest === undefined) {
    reject("artifact does not contain the policy-pinned package manifest");
  }

  const embedded = parseJsonStrict(challengeEntry.bytes, "embedded challenge", {
    maxBytes: MAX_CHALLENGE_BYTES,
  });
  if (!canonicalize(embedded).equals(canonicalize(expectedChallenge))) {
    reject("embedded challenge does not match the signed request challenge");
  }
  const manifestValue = parseJsonStrict(manifest, "npm package manifest", {
    maxBytes: MAX_MANIFEST_BYTES,
  });
  if (
    manifestValue === null ||
    Array.isArray(manifestValue) ||
    typeof manifestValue !== "object" ||
    manifestValue.name !== packageName ||
    manifestValue.version !== packageVersion
  ) {
    reject("npm package manifest does not bind the exact package and version");
  }

  return {
    challengeBytes: Buffer.from(challengeEntry.bytes),
    manifestBytes: Buffer.from(manifest),
  };
}
