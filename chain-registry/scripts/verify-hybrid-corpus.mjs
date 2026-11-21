#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  ED25519_PUBLIC_KEY_LEN,
  HYBRID_DOMAIN_PREFIX,
  HYBRID_MLDSA_CONTEXT,
  HYBRID_SCHEME_ID,
  MLDSA44_PUBLIC_KEY_LEN,
  computeHybridKeyId,
  strictHexToBytes,
  verifyHybridSignatureConcat,
  verifyHybridSignatureHex,
  verifyMlDsa44Pure,
} from "./hybrid-verifier/verifier.mjs";

const scriptsDir = dirname(fileURLToPath(import.meta.url));

export const DEFAULT_CORPUS_PATH = resolve(
  scriptsDir,
  "../crates/common/tests/vectors/hybrid_differential_corpus.json",
);
export const DEFAULT_ACVP_SIGVER_PATH = resolve(
  scriptsDir,
  "../crates/common/tests/vectors/mldsa44_sigver_acvp.json",
);

const EXPECTED_DOMAIN_PREFIX_STR = new TextDecoder().decode(HYBRID_DOMAIN_PREFIX);
const EXPECTED_CONTEXT_STR = new TextDecoder().decode(HYBRID_MLDSA_CONTEXT);

function concatBytes(left, right) {
  const out = new Uint8Array(left.length + right.length);
  out.set(left, 0);
  out.set(right, left.length);
  return out;
}

function readJsonFile(filePath, label) {
  let raw;
  try {
    raw = readFileSync(filePath, "utf8");
  } catch (error) {
    throw new Error(`Failed to read ${label} (${filePath}): ${error.message}`);
  }
  try {
    return JSON.parse(raw);
  } catch (error) {
    throw new Error(`Failed to parse ${label} JSON (${filePath}): ${error.message}`);
  }
}

/**
 * Verify every entry in a Rust-generated hybrid differential corpus JSON object.
 */
export function verifyDifferentialCorpus(corpus) {
  if (corpus === null || typeof corpus !== "object" || Array.isArray(corpus)) {
    throw new Error("Differential corpus root must be a JSON object");
  }
  if (corpus.schema_version !== 1) {
    throw new Error(
      `Unsupported corpus schema_version: expected 1, got ${JSON.stringify(corpus.schema_version)}`,
    );
  }
  if (corpus.scheme !== HYBRID_SCHEME_ID) {
    throw new Error(
      `Unsupported corpus scheme: expected ${HYBRID_SCHEME_ID}, got ${JSON.stringify(corpus.scheme)}`,
    );
  }
  if (corpus.domain_prefix !== EXPECTED_DOMAIN_PREFIX_STR) {
    throw new Error(
      `Unexpected corpus domain_prefix: expected ${EXPECTED_DOMAIN_PREFIX_STR}, got ${JSON.stringify(corpus.domain_prefix)}`,
    );
  }
  if (corpus.mldsa44_context !== EXPECTED_CONTEXT_STR) {
    throw new Error(
      `Unexpected corpus mldsa44_context: expected ${EXPECTED_CONTEXT_STR}, got ${JSON.stringify(corpus.mldsa44_context)}`,
    );
  }
  if (!Array.isArray(corpus.cases) || corpus.cases.length === 0) {
    throw new Error("Differential corpus must contain a non-empty cases array");
  }

  const seenIds = new Set();
  const mismatches = [];
  let validCount = 0;
  let invalidCount = 0;

  for (let idx = 0; idx < corpus.cases.length; idx++) {
    const entry = corpus.cases[idx];
    if (entry === null || typeof entry !== "object" || Array.isArray(entry)) {
      throw new Error(`Corpus case at index ${idx} must be an object`);
    }
    if (typeof entry.id !== "string" || entry.id.length === 0) {
      throw new Error(`Corpus case at index ${idx} must have a non-empty string id`);
    }
    if (seenIds.has(entry.id)) {
      throw new Error(`Duplicate corpus case id: ${entry.id}`);
    }
    seenIds.add(entry.id);

    if (typeof entry.expected_valid !== "boolean") {
      throw new Error(`Corpus case ${entry.id} must have a boolean expected_valid`);
    }
    if (entry.expected_key_id !== null && typeof entry.expected_key_id !== "string") {
      throw new Error(
        `Corpus case ${entry.id} must have expected_key_id as string or null`,
      );
    }

    const hexVerdict = verifyHybridSignatureHex({
      scheme: corpus.scheme,
      ed25519PkHex: entry.ed25519_pk,
      mldsa44PkHex: entry.mldsa44_pk,
      ed25519SigHex: entry.ed25519_sig,
      mldsa44SigHex: entry.mldsa44_sig,
      messageHex: entry.message,
    });

    if (hexVerdict.valid !== entry.expected_valid) {
      mismatches.push({
        id: entry.id,
        check: "hex_parts",
        expected: entry.expected_valid,
        actual: hexVerdict.valid,
        reason: hexVerdict.reason ?? "accepted_unexpectedly",
      });
    }

    const edPkBytes = strictHexToBytes(entry.ed25519_pk);
    const pqPkBytes = strictHexToBytes(entry.mldsa44_pk);
    const edSigBytes = strictHexToBytes(entry.ed25519_sig);
    const pqSigBytes = strictHexToBytes(entry.mldsa44_sig);
    const msgBytes = strictHexToBytes(entry.message);

    if (
      edPkBytes !== null &&
      pqPkBytes !== null &&
      edSigBytes !== null &&
      pqSigBytes !== null &&
      msgBytes !== null
    ) {
      const concatVerdict = verifyHybridSignatureConcat({
        publicKey: concatBytes(edPkBytes, pqPkBytes),
        signature: concatBytes(edSigBytes, pqSigBytes),
        message: msgBytes,
      });
      if (concatVerdict.valid !== entry.expected_valid) {
        mismatches.push({
          id: entry.id,
          check: "concat",
          expected: entry.expected_valid,
          actual: concatVerdict.valid,
          reason: concatVerdict.reason ?? "accepted_unexpectedly",
        });
      }
    }

    if (
      edPkBytes !== null &&
      edPkBytes.length === ED25519_PUBLIC_KEY_LEN &&
      pqPkBytes !== null &&
      pqPkBytes.length === MLDSA44_PUBLIC_KEY_LEN
    ) {
      const actualKeyId = computeHybridKeyId(edPkBytes, pqPkBytes);
      if (actualKeyId !== entry.expected_key_id) {
        mismatches.push({
          id: entry.id,
          check: "key_id",
          expected: entry.expected_key_id,
          actual: actualKeyId,
          reason: "key_id_mismatch",
        });
      }
    } else if (entry.expected_key_id !== null) {
      mismatches.push({
        id: entry.id,
        check: "key_id",
        expected: entry.expected_key_id,
        actual: null,
        reason: "expected_key_id must be null when public key lengths are invalid",
      });
    }

    if (entry.expected_valid) {
      validCount++;
    } else {
      invalidCount++;
    }
  }

  return {
    ok: mismatches.length === 0,
    schemaVersion: corpus.schema_version,
    scheme: corpus.scheme,
    total: corpus.cases.length,
    validCount,
    invalidCount,
    mismatches,
  };
}

/**
 * Verify NIST ACVP ML-DSA-44 sigVer cases against the pinned JS ML-DSA-44 verifier.
 */
export function verifyAcvpSigVer(acvp) {
  if (acvp === null || typeof acvp !== "object" || Array.isArray(acvp)) {
    throw new Error("ACVP sigVer root must be a JSON object");
  }
  if (acvp.parameterSet !== "ML-DSA-44") {
    throw new Error(
      `Unexpected ACVP parameterSet: expected ML-DSA-44, got ${JSON.stringify(acvp.parameterSet)}`,
    );
  }
  if (!Array.isArray(acvp.cases) || acvp.cases.length === 0) {
    throw new Error("ACVP sigVer fixture must contain a non-empty cases array");
  }

  const mismatches = [];
  let passedCount = 0;
  let failedCount = 0;

  for (const testCase of acvp.cases) {
    if (typeof testCase.testPassed !== "boolean") {
      throw new Error(`ACVP case tcId=${testCase.tcId} missing boolean testPassed`);
    }
    const pk = strictHexToBytes(String(testCase.pk ?? "").toLowerCase());
    const message = strictHexToBytes(String(testCase.message ?? "").toLowerCase());
    const context = strictHexToBytes(String(testCase.context ?? "").toLowerCase());
    const signature = strictHexToBytes(String(testCase.signature ?? "").toLowerCase());

    let actual = false;
    if (pk !== null && message !== null && context !== null && signature !== null) {
      actual = verifyMlDsa44Pure(signature, message, pk, context);
    }

    if (actual !== testCase.testPassed) {
      mismatches.push({
        tcId: testCase.tcId,
        expected: testCase.testPassed,
        actual,
        reason: "acvp_sigver_mismatch",
      });
    }

    if (testCase.testPassed) {
      passedCount++;
    } else {
      failedCount++;
    }
  }

  return {
    ok: mismatches.length === 0,
    parameterSet: acvp.parameterSet,
    acvpCommit: acvp.acvpCommit ?? null,
    total: acvp.cases.length,
    passedCount,
    failedCount,
    mismatches,
  };
}

export function verifyCorpusFile(corpusPath = DEFAULT_CORPUS_PATH) {
  return verifyDifferentialCorpus(readJsonFile(corpusPath, "differential corpus"));
}

export function verifyAcvpFile(acvpPath = DEFAULT_ACVP_SIGVER_PATH) {
  return verifyAcvpSigVer(readJsonFile(acvpPath, "ACVP sigVer fixture"));
}

function usage() {
  console.log(`Usage: node scripts/verify-hybrid-corpus.mjs [OPTIONS]

Verifies the Rust-generated hybrid-ed25519-mldsa44-v1 differential corpus and
NIST ACVP ML-DSA-44 sigVer vectors using the pinned @noble/post-quantum JS verifier.

Options:
  --corpus PATH    Path to differential corpus JSON (default: ${DEFAULT_CORPUS_PATH})
  --acvp PATH      Path to ACVP sigVer JSON (default: ${DEFAULT_ACVP_SIGVER_PATH})
  --skip-acvp      Skip the ACVP sigVer cross-check
  --json           Emit machine-readable JSON summary on stdout
  -h, --help       Show this help text`);
}

function parseArgs(argv) {
  let corpusPath = DEFAULT_CORPUS_PATH;
  let acvpPath = DEFAULT_ACVP_SIGVER_PATH;
  let skipAcvp = false;
  let jsonOutput = false;
  let showHelp = false;

  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--corpus") {
      const value = argv[++i];
      if (!value) throw new Error("--corpus requires a path");
      corpusPath = resolve(value);
    } else if (arg === "--acvp") {
      const value = argv[++i];
      if (!value) throw new Error("--acvp requires a path");
      acvpPath = resolve(value);
    } else if (arg === "--skip-acvp") {
      skipAcvp = true;
    } else if (arg === "--json") {
      jsonOutput = true;
    } else if (arg === "-h" || arg === "--help") {
      showHelp = true;
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }

  return { corpusPath, acvpPath, skipAcvp, jsonOutput, showHelp };
}

function main() {
  const args = parseArgs(process.argv);
  if (args.showHelp) {
    usage();
    return;
  }

  const corpusResult = verifyCorpusFile(args.corpusPath);
  const acvpResult = args.skipAcvp ? null : verifyAcvpFile(args.acvpPath);
  const ok = corpusResult.ok && (acvpResult ? acvpResult.ok : true);

  if (args.jsonOutput) {
    console.log(
      JSON.stringify(
        {
          ok,
          corpus: corpusResult,
          acvpSigVer: acvpResult,
        },
        null,
        2,
      ),
    );
  } else {
    console.log(
      `Hybrid differential corpus (${corpusResult.scheme}): ${corpusResult.total - corpusResult.mismatches.length}/${corpusResult.total} matched (${corpusResult.validCount} valid, ${corpusResult.invalidCount} invalid)`,
    );
    if (acvpResult !== null) {
      console.log(
        `ACVP ML-DSA-44 sigVer: ${acvpResult.total - acvpResult.mismatches.length}/${acvpResult.total} matched (${acvpResult.passedCount} pass, ${acvpResult.failedCount} fail)`,
      );
    }
  }

  if (!ok) {
    for (const m of corpusResult.mismatches) {
      console.error(
        `Corpus mismatch [${m.id}] (${m.check}): expected=${m.expected} actual=${m.actual} (${m.reason})`,
      );
    }
    if (acvpResult !== null) {
      for (const m of acvpResult.mismatches) {
        console.error(
          `ACVP sigVer mismatch [tcId=${m.tcId}]: expected=${m.expected} actual=${m.actual}`,
        );
      }
    }
    process.exitCode = 1;
  }
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
