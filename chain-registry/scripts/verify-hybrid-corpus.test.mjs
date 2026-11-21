import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  ED25519_PUBLIC_KEY_LEN,
  ED25519_SIGNATURE_LEN,
  HYBRID_PUBLIC_KEY_LEN,
  HYBRID_SCHEME_ID,
  HYBRID_SIGNATURE_LEN,
  MLDSA44_PUBLIC_KEY_LEN,
  MLDSA44_SIGNATURE_LEN,
  buildHybridMessage,
  bytesToHex,
  mldsa44KeygenForTest,
  mldsa44SignForTest,
  strictHexToBytes,
  verifyEd25519CofactoredForTest,
  verifyEd25519Strict,
  verifyHybridSignatureConcat,
  verifyHybridSignatureHex,
} from "./hybrid-verifier/verifier.mjs";
import {
  DEFAULT_ACVP_SIGVER_PATH,
  DEFAULT_CORPUS_PATH,
  verifyAcvpSigVer,
  verifyCorpusFile,
  verifyDifferentialCorpus,
} from "./verify-hybrid-corpus.mjs";

const scriptsDir = dirname(fileURLToPath(import.meta.url));
const chainRegistryDir = resolve(scriptsDir, "..");
const vectorsDir = resolve(chainRegistryDir, "crates/common/tests/vectors");
const cliScriptPath = resolve(scriptsDir, "verify-hybrid-corpus.mjs");

function hexLenientToBytes(hex) {
  return strictHexToBytes(String(hex).toLowerCase());
}

test("checked-in hybrid_differential_corpus.json agrees with Rust on all 34 cases", () => {
  const result = verifyCorpusFile(DEFAULT_CORPUS_PATH);
  assert.equal(
    result.ok,
    true,
    `Unexpected differential mismatches: ${JSON.stringify(result.mismatches)}`,
  );
  assert.equal(result.scheme, HYBRID_SCHEME_ID);
  assert.equal(result.total, 34);
  assert.equal(result.validCount, 5);
  assert.equal(result.invalidCount, 29);

  const corpus = JSON.parse(readFileSync(DEFAULT_CORPUS_PATH, "utf8"));
  const hedgedA = corpus.cases.find((c) => c.id === "valid_hedged_rnd_a");
  const hedgedB = corpus.cases.find((c) => c.id === "valid_hedged_rnd_b");
  assert.ok(hedgedA && hedgedB);
  assert.equal(hedgedA.ed25519_pk, hedgedB.ed25519_pk);
  assert.equal(hedgedA.mldsa44_pk, hedgedB.mldsa44_pk);
  assert.equal(hedgedA.ed25519_sig, hedgedB.ed25519_sig);
  assert.equal(hedgedA.message, hedgedB.message);
  assert.equal(hedgedA.expected_key_id, hedgedB.expected_key_id);
  assert.notEqual(hedgedA.mldsa44_sig, hedgedB.mldsa44_sig);
});

test("flipping expected_valid on every single corpus case or key_id triggers a mismatch", () => {
  const baseCorpus = JSON.parse(readFileSync(DEFAULT_CORPUS_PATH, "utf8"));

  for (let idx = 0; idx < baseCorpus.cases.length; idx++) {
    const mutated = structuredClone(baseCorpus);
    const targetCase = mutated.cases[idx];
    targetCase.expected_valid = !targetCase.expected_valid;

    const result = verifyDifferentialCorpus(mutated);
    assert.equal(result.ok, false, `Flipping case ${targetCase.id} must fail`);
    assert.ok(
      result.mismatches.some((m) => m.id === targetCase.id),
      `Expected mismatch on ${targetCase.id}`,
    );
  }

  const badKeyIdCorpus = structuredClone(baseCorpus);
  const validCase = badKeyIdCorpus.cases.find((c) => c.expected_valid);
  validCase.expected_key_id =
    (validCase.expected_key_id[0] === "0" ? "1" : "0") + validCase.expected_key_id.slice(1);
  const badKeyIdResult = verifyDifferentialCorpus(badKeyIdCorpus);
  assert.equal(badKeyIdResult.ok, false);
  assert.ok(
    badKeyIdResult.mismatches.some((m) => m.id === validCase.id && m.check === "key_id"),
  );

  const unexpectedKeyIdCorpus = structuredClone(baseCorpus);
  const shortPkCase = unexpectedKeyIdCorpus.cases.find(
    (c) => c.id === "wrong_ed25519_pk_len_short",
  );
  shortPkCase.expected_key_id = "00".repeat(32);
  const unexpectedKeyIdResult = verifyDifferentialCorpus(unexpectedKeyIdCorpus);
  assert.equal(unexpectedKeyIdResult.ok, false);
  assert.ok(
    unexpectedKeyIdResult.mismatches.some(
      (m) => m.id === shortPkCase.id && m.check === "key_id",
    ),
  );
});

test("strict Ed25519 rejects cofactored small-order R forgery accepted by cofactored verify", () => {
  const corpus = JSON.parse(readFileSync(DEFAULT_CORPUS_PATH, "utf8"));
  const cofactoredCase = corpus.cases.find(
    (c) => c.id === "ed25519_small_order_r_cofactored_forgery_with_valid_mldsa",
  );
  assert.ok(cofactoredCase);

  const edPk = strictHexToBytes(cofactoredCase.ed25519_pk, ED25519_PUBLIC_KEY_LEN);
  const edSig = strictHexToBytes(cofactoredCase.ed25519_sig, ED25519_SIGNATURE_LEN);
  const msg = strictHexToBytes(cofactoredCase.message);
  const mPrime = buildHybridMessage(msg);

  // Demonstrate why direct `ed25519.verify(..., { zip215: false })` would diverge from ed25519-dalek:
  assert.equal(verifyEd25519CofactoredForTest(edSig, mPrime, edPk), true);
  // Our strict cofactorless verifier must reject it:
  assert.equal(verifyEd25519Strict(edSig, mPrime, edPk), false);

  const verdict = verifyHybridSignatureHex({
    scheme: HYBRID_SCHEME_ID,
    ed25519PkHex: cofactoredCase.ed25519_pk,
    mldsa44PkHex: cofactoredCase.mldsa44_pk,
    ed25519SigHex: cofactoredCase.ed25519_sig,
    mldsa44SigHex: cofactoredCase.mldsa44_sig,
    messageHex: cofactoredCase.message,
  });
  assert.equal(verdict.valid, false);
  assert.equal(verdict.reason, "ed25519_verification_failed");
});

test("NIST ACVP ML-DSA-44 keyGen, sigGen (det + hedged), and sigVer vectors match in JS", () => {
  const sigVerJson = JSON.parse(readFileSync(DEFAULT_ACVP_SIGVER_PATH, "utf8"));
  const sigVerRes = verifyAcvpSigVer(sigVerJson);
  assert.equal(
    sigVerRes.ok,
    true,
    `ACVP sigVer mismatches: ${JSON.stringify(sigVerRes.mismatches)}`,
  );
  assert.equal(sigVerRes.total, 15);
  assert.equal(sigVerRes.passedCount, 3);
  assert.equal(sigVerRes.failedCount, 12);

  const keyGenJson = JSON.parse(
    readFileSync(resolve(vectorsDir, "mldsa44_keygen_acvp.json"), "utf8"),
  );
  assert.equal(keyGenJson.cases.length, 5);
  for (const tc of keyGenJson.cases) {
    const { publicKey, secretKey } = mldsa44KeygenForTest(hexLenientToBytes(tc.seed));
    assert.equal(bytesToHex(publicKey), tc.pk.toLowerCase(), `keyGen pk tcId=${tc.tcId}`);
    assert.equal(bytesToHex(secretKey), tc.sk.toLowerCase(), `keyGen sk tcId=${tc.tcId}`);
  }

  const sigGenJson = JSON.parse(
    readFileSync(resolve(vectorsDir, "mldsa44_siggen_acvp.json"), "utf8"),
  );
  assert.equal(sigGenJson.deterministicPure.length, 5);
  for (const tc of sigGenJson.deterministicPure) {
    const sig = mldsa44SignForTest(
      hexLenientToBytes(tc.message),
      hexLenientToBytes(tc.sk),
      { context: hexLenientToBytes(tc.context), extraEntropy: false },
    );
    assert.equal(
      bytesToHex(sig),
      tc.signature.toLowerCase(),
      `deterministic sigGen tcId=${tc.tcId}`,
    );
  }
  assert.equal(sigGenJson.hedgedPure.length, 2);
  for (const tc of sigGenJson.hedgedPure) {
    const sig = mldsa44SignForTest(
      hexLenientToBytes(tc.message),
      hexLenientToBytes(tc.sk),
      { context: hexLenientToBytes(tc.context), extraEntropy: hexLenientToBytes(tc.rnd) },
    );
    assert.equal(
      bytesToHex(sig),
      tc.signature.toLowerCase(),
      `hedged sigGen tcId=${tc.tcId}`,
    );
  }
});

test("verifyHybridSignatureHex and Concat enforce strict hex, scheme, and exact lengths", () => {
  const corpus = JSON.parse(readFileSync(DEFAULT_CORPUS_PATH, "utf8"));
  const valid = corpus.cases.find((c) => c.id === "valid_pbft_commit_deterministic");
  assert.ok(valid);

  const baseArgs = {
    scheme: HYBRID_SCHEME_ID,
    ed25519PkHex: valid.ed25519_pk,
    mldsa44PkHex: valid.mldsa44_pk,
    ed25519SigHex: valid.ed25519_sig,
    mldsa44SigHex: valid.mldsa44_sig,
    messageHex: valid.message,
  };
  assert.deepEqual(verifyHybridSignatureHex(baseArgs), { valid: true });

  for (const badScheme of ["ed25519", "", "Hybrid-Ed25519-MlDsa44-V1", null]) {
    const res = verifyHybridSignatureHex({ ...baseArgs, scheme: badScheme });
    assert.equal(res.valid, false);
    assert.equal(res.reason, "scheme_mismatch");
  }

  for (const field of [
    "ed25519PkHex",
    "mldsa44PkHex",
    "ed25519SigHex",
    "mldsa44SigHex",
    "messageHex",
  ]) {
    const orig = baseArgs[field];
    assert.equal(verifyHybridSignatureHex({ ...baseArgs, [field]: orig.toUpperCase() }).valid, false);
    assert.equal(verifyHybridSignatureHex({ ...baseArgs, [field]: `0x${orig}` }).valid, false);
    assert.equal(verifyHybridSignatureHex({ ...baseArgs, [field]: `${orig}0` }).valid, false);
    assert.equal(verifyHybridSignatureHex({ ...baseArgs, [field]: ` ${orig} ` }).valid, false);
    assert.equal(verifyHybridSignatureHex({ ...baseArgs, [field]: 123 }).valid, false);
  }

  const pkBytes = new Uint8Array(HYBRID_PUBLIC_KEY_LEN);
  pkBytes.set(strictHexToBytes(valid.ed25519_pk, ED25519_PUBLIC_KEY_LEN), 0);
  pkBytes.set(strictHexToBytes(valid.mldsa44_pk, MLDSA44_PUBLIC_KEY_LEN), ED25519_PUBLIC_KEY_LEN);

  const sigBytes = new Uint8Array(HYBRID_SIGNATURE_LEN);
  sigBytes.set(strictHexToBytes(valid.ed25519_sig, ED25519_SIGNATURE_LEN), 0);
  sigBytes.set(strictHexToBytes(valid.mldsa44_sig, MLDSA44_SIGNATURE_LEN), ED25519_SIGNATURE_LEN);

  const msgBytes = strictHexToBytes(valid.message);
  assert.deepEqual(
    verifyHybridSignatureConcat({ publicKey: pkBytes, signature: sigBytes, message: msgBytes }),
    { valid: true },
  );

  for (const badPk of [
    pkBytes.subarray(0, HYBRID_PUBLIC_KEY_LEN - 1),
    new Uint8Array(HYBRID_PUBLIC_KEY_LEN + 1),
  ]) {
    const res = verifyHybridSignatureConcat({
      publicKey: badPk,
      signature: sigBytes,
      message: msgBytes,
    });
    assert.equal(res.valid, false);
    assert.equal(res.reason, "invalid_hybrid_pk_length");
  }

  for (const badSig of [
    sigBytes.subarray(0, HYBRID_SIGNATURE_LEN - 1),
    new Uint8Array(HYBRID_SIGNATURE_LEN + 1),
  ]) {
    const res = verifyHybridSignatureConcat({
      publicKey: pkBytes,
      signature: badSig,
      message: msgBytes,
    });
    assert.equal(res.valid, false);
    assert.equal(res.reason, "invalid_hybrid_sig_length");
  }
});

test("verifyEd25519Strict rejects all 8 small-order public keys and non-canonical field y = p", () => {
  const smallOrderKeys = [
    "0100000000000000000000000000000000000000000000000000000000000000", // identity (order 1)
    "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f", // order 2
    "0000000000000000000000000000000000000000000000000000000000000000", // order 4
    "0000000000000000000000000000000000000000000000000000000000000080", // order 4
    "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a", // order 8
    "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac03fa", // order 8
    "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05", // order 8
    "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc85", // order 8
  ];
  const forgedSig = new Uint8Array(ED25519_SIGNATURE_LEN);
  forgedSig[0] = 1; // R = identity, S = 0
  const mPrime = buildHybridMessage(new TextEncoder().encode("small-order-test"));

  for (const hexPk of smallOrderKeys) {
    const pk = strictHexToBytes(hexPk, ED25519_PUBLIC_KEY_LEN);
    assert.equal(verifyEd25519Strict(forgedSig, mPrime, pk), false, `must reject ${hexPk}`);
  }

  const nonCanonicalY = strictHexToBytes(
    "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
    ED25519_PUBLIC_KEY_LEN,
  );
  assert.equal(verifyEd25519Strict(forgedSig, mPrime, nonCanonicalY), false);
});

test("verify-hybrid-corpus CLI exits 0 on --json and --help", () => {
  const jsonOut = execFileSync(process.execPath, [cliScriptPath, "--json"], {
    cwd: chainRegistryDir,
    encoding: "utf8",
  });
  const parsed = JSON.parse(jsonOut);
  assert.equal(parsed.ok, true);
  assert.equal(parsed.corpus.total, 34);
  assert.equal(parsed.acvpSigVer.total, 15);

  const helpOut = execFileSync(process.execPath, [cliScriptPath, "--help"], {
    cwd: chainRegistryDir,
    encoding: "utf8",
  });
  assert.match(helpOut, /--corpus/);
  assert.match(helpOut, /--acvp/);
});
