import { ml_dsa44 } from "@noble/post-quantum/ml-dsa.js";
import { ed25519 } from "@noble/curves/ed25519.js";
import { sha256, sha512 } from "@noble/hashes/sha2.js";

export const HYBRID_SCHEME_ID = "hybrid-ed25519-mldsa44-v1";
export const HYBRID_DOMAIN_PREFIX = new TextEncoder().encode("creg-hybrid-sig-v1");
export const HYBRID_MLDSA_CONTEXT = new TextEncoder().encode("creg-hybrid-sig-v1");
export const KEY_ID_DOMAIN_PREFIX = new TextEncoder().encode("creg-key-id-v1");

export const ED25519_PUBLIC_KEY_LEN = 32;
export const ED25519_SIGNATURE_LEN = 64;
export const MLDSA44_PUBLIC_KEY_LEN = 1312;
export const MLDSA44_SIGNATURE_LEN = 2420;
export const HYBRID_PUBLIC_KEY_LEN = ED25519_PUBLIC_KEY_LEN + MLDSA44_PUBLIC_KEY_LEN;
export const HYBRID_SIGNATURE_LEN = ED25519_SIGNATURE_LEN + MLDSA44_SIGNATURE_LEN;

const LOWER_HEX_RE = /^[0-9a-f]*$/u;

/**
 * Decode strict lowercase ASCII hex without `0x` prefix or whitespace.
 * Returns `null` on invalid format or length mismatch.
 */
export function strictHexToBytes(hex, expectedByteLen = null) {
  if (typeof hex !== "string" || hex.length % 2 !== 0 || !LOWER_HEX_RE.test(hex)) {
    return null;
  }
  const byteLen = hex.length / 2;
  if (expectedByteLen !== null && byteLen !== expectedByteLen) {
    return null;
  }
  const out = new Uint8Array(byteLen);
  for (let i = 0; i < byteLen; i++) {
    out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

/**
 * Encode bytes as lowercase ASCII hex without a `0x` prefix.
 */
export function bytesToHex(bytes) {
  if (!(bytes instanceof Uint8Array)) {
    throw new TypeError("bytes must be a Uint8Array");
  }
  let out = "";
  for (let i = 0; i < bytes.length; i++) {
    out += bytes[i].toString(16).padStart(2, "0");
  }
  return out;
}

/**
 * Build the composite domain-separated message M' = "creg-hybrid-sig-v1" || 0x00 || M.
 */
export function buildHybridMessage(messageBytes) {
  if (!(messageBytes instanceof Uint8Array)) {
    throw new TypeError("messageBytes must be a Uint8Array");
  }
  const out = new Uint8Array(HYBRID_DOMAIN_PREFIX.length + 1 + messageBytes.length);
  out.set(HYBRID_DOMAIN_PREFIX, 0);
  out[HYBRID_DOMAIN_PREFIX.length] = 0x00;
  out.set(messageBytes, HYBRID_DOMAIN_PREFIX.length + 1);
  return out;
}

/**
 * Compute ADR-017 §3 key identifier:
 * sha256("creg-key-id-v1" || 0x00 || ed25519_pk || mldsa44_pk) as lowercase hex.
 */
export function computeHybridKeyId(ed25519PkBytes, mldsa44PkBytes) {
  if (!(ed25519PkBytes instanceof Uint8Array) || ed25519PkBytes.length !== ED25519_PUBLIC_KEY_LEN) {
    throw new RangeError(`ed25519PkBytes must be a ${ED25519_PUBLIC_KEY_LEN}-byte Uint8Array`);
  }
  if (!(mldsa44PkBytes instanceof Uint8Array) || mldsa44PkBytes.length !== MLDSA44_PUBLIC_KEY_LEN) {
    throw new RangeError(`mldsa44PkBytes must be a ${MLDSA44_PUBLIC_KEY_LEN}-byte Uint8Array`);
  }
  const input = new Uint8Array(
    KEY_ID_DOMAIN_PREFIX.length + 1 + ED25519_PUBLIC_KEY_LEN + MLDSA44_PUBLIC_KEY_LEN,
  );
  input.set(KEY_ID_DOMAIN_PREFIX, 0);
  input[KEY_ID_DOMAIN_PREFIX.length] = 0x00;
  input.set(ed25519PkBytes, KEY_ID_DOMAIN_PREFIX.length + 1);
  input.set(mldsa44PkBytes, KEY_ID_DOMAIN_PREFIX.length + 1 + ED25519_PUBLIC_KEY_LEN);
  return bytesToHex(sha256(input));
}

function bytesToNumberLE(bytes) {
  let acc = 0n;
  for (let i = bytes.length - 1; i >= 0; i--) {
    acc = (acc << 8n) + BigInt(bytes[i]);
  }
  return acc;
}

/**
 * Strict cofactorless Ed25519 verification matching `ed25519_dalek::VerifyingKey::verify_strict`:
 * - Exact 32-byte public key and 64-byte signature
 * - Canonical scalar S < L
 * - Canonical RFC 8032 / NIST point decompression for A and R (zip215 = false)
 * - Rejects small-order public keys A AND small-order signature commitments R
 * - Checks cofactorless equation [S]B - [k]A == R
 */
export function verifyEd25519Strict(sigBytes, messageBytes, pkBytes) {
  if (
    !(sigBytes instanceof Uint8Array) ||
    sigBytes.length !== ED25519_SIGNATURE_LEN ||
    !(pkBytes instanceof Uint8Array) ||
    pkBytes.length !== ED25519_PUBLIC_KEY_LEN ||
    !(messageBytes instanceof Uint8Array)
  ) {
    return false;
  }

  const rBytes = sigBytes.subarray(0, 32);
  const s = bytesToNumberLE(sigBytes.subarray(32, 64));
  const L = ed25519.Point.Fn.ORDER;
  if (s >= L) {
    return false;
  }

  let A;
  let R;
  try {
    A = ed25519.Point.fromBytes(pkBytes, false);
    R = ed25519.Point.fromBytes(rBytes, false);
  } catch {
    return false;
  }

  if (A.isSmallOrder() || R.isSmallOrder()) {
    return false;
  }

  const digestInput = new Uint8Array(64 + messageBytes.length);
  digestInput.set(rBytes, 0);
  digestInput.set(pkBytes, 32);
  digestInput.set(messageBytes, 64);
  const k = bytesToNumberLE(sha512(digestInput)) % L;

  const SB = s === 0n ? ed25519.Point.ZERO : ed25519.Point.BASE.multiplyUnsafe(s);
  const kA = k === 0n ? ed25519.Point.ZERO : A.multiplyUnsafe(k);
  const expectedR = SB.subtract(kA).toBytes();

  let diff = 0;
  for (let i = 0; i < 32; i++) {
    diff |= expectedR[i] ^ rBytes[i];
  }
  return diff === 0;
}

/**
 * Pure FIPS 204 ML-DSA-44 verification with explicit length checks before parsing.
 */
export function verifyMlDsa44Pure(
  sigBytes,
  messageBytes,
  pkBytes,
  contextBytes = HYBRID_MLDSA_CONTEXT,
) {
  if (
    !(sigBytes instanceof Uint8Array) ||
    sigBytes.length !== MLDSA44_SIGNATURE_LEN ||
    !(pkBytes instanceof Uint8Array) ||
    pkBytes.length !== MLDSA44_PUBLIC_KEY_LEN ||
    !(messageBytes instanceof Uint8Array) ||
    !(contextBytes instanceof Uint8Array)
  ) {
    return false;
  }
  try {
    return Boolean(ml_dsa44.verify(sigBytes, messageBytes, pkBytes, { context: contextBytes }));
  } catch {
    return false;
  }
}

/**
 * Verify a CREG hybrid-ed25519-mldsa44-v1 signature from split byte components.
 */
export function verifyHybridSignatureParts({
  ed25519Pk,
  mldsa44Pk,
  ed25519Sig,
  mldsa44Sig,
  message,
}) {
  if (!(ed25519Pk instanceof Uint8Array) || ed25519Pk.length !== ED25519_PUBLIC_KEY_LEN) {
    return { valid: false, reason: "invalid_ed25519_pk_length" };
  }
  if (!(mldsa44Pk instanceof Uint8Array) || mldsa44Pk.length !== MLDSA44_PUBLIC_KEY_LEN) {
    return { valid: false, reason: "invalid_mldsa44_pk_length" };
  }
  if (!(ed25519Sig instanceof Uint8Array) || ed25519Sig.length !== ED25519_SIGNATURE_LEN) {
    return { valid: false, reason: "invalid_ed25519_sig_length" };
  }
  if (!(mldsa44Sig instanceof Uint8Array) || mldsa44Sig.length !== MLDSA44_SIGNATURE_LEN) {
    return { valid: false, reason: "invalid_mldsa44_sig_length" };
  }
  if (!(message instanceof Uint8Array)) {
    return { valid: false, reason: "invalid_message" };
  }

  const mPrime = buildHybridMessage(message);
  if (!verifyEd25519Strict(ed25519Sig, mPrime, ed25519Pk)) {
    return { valid: false, reason: "ed25519_verification_failed" };
  }
  if (!verifyMlDsa44Pure(mldsa44Sig, mPrime, mldsa44Pk, HYBRID_MLDSA_CONTEXT)) {
    return { valid: false, reason: "mldsa44_verification_failed" };
  }
  return { valid: true };
}

/**
 * Verify a CREG hybrid-ed25519-mldsa44-v1 signature from canonical concatenated bytes.
 */
export function verifyHybridSignatureConcat({ publicKey, signature, message }) {
  if (!(publicKey instanceof Uint8Array) || publicKey.length !== HYBRID_PUBLIC_KEY_LEN) {
    return { valid: false, reason: "invalid_hybrid_pk_length" };
  }
  if (!(signature instanceof Uint8Array) || signature.length !== HYBRID_SIGNATURE_LEN) {
    return { valid: false, reason: "invalid_hybrid_sig_length" };
  }
  if (!(message instanceof Uint8Array)) {
    return { valid: false, reason: "invalid_message" };
  }

  return verifyHybridSignatureParts({
    ed25519Pk: publicKey.subarray(0, ED25519_PUBLIC_KEY_LEN),
    mldsa44Pk: publicKey.subarray(ED25519_PUBLIC_KEY_LEN),
    ed25519Sig: signature.subarray(0, ED25519_SIGNATURE_LEN),
    mldsa44Sig: signature.subarray(ED25519_SIGNATURE_LEN),
    message,
  });
}

/**
 * Verify a CREG hybrid-ed25519-mldsa44-v1 signature from strict lowercase hex strings.
 */
export function verifyHybridSignatureHex({
  scheme,
  ed25519PkHex,
  mldsa44PkHex,
  ed25519SigHex,
  mldsa44SigHex,
  messageHex,
}) {
  if (scheme !== HYBRID_SCHEME_ID) {
    return { valid: false, reason: "scheme_mismatch" };
  }

  const ed25519Pk = strictHexToBytes(ed25519PkHex, ED25519_PUBLIC_KEY_LEN);
  if (ed25519Pk === null) {
    return { valid: false, reason: "invalid_ed25519_pk_hex_or_length" };
  }
  const mldsa44Pk = strictHexToBytes(mldsa44PkHex, MLDSA44_PUBLIC_KEY_LEN);
  if (mldsa44Pk === null) {
    return { valid: false, reason: "invalid_mldsa44_pk_hex_or_length" };
  }
  const ed25519Sig = strictHexToBytes(ed25519SigHex, ED25519_SIGNATURE_LEN);
  if (ed25519Sig === null) {
    return { valid: false, reason: "invalid_ed25519_sig_hex_or_length" };
  }
  const mldsa44Sig = strictHexToBytes(mldsa44SigHex, MLDSA44_SIGNATURE_LEN);
  if (mldsa44Sig === null) {
    return { valid: false, reason: "invalid_mldsa44_sig_hex_or_length" };
  }
  const message = strictHexToBytes(messageHex);
  if (message === null) {
    return { valid: false, reason: "invalid_message_hex" };
  }

  return verifyHybridSignatureParts({
    ed25519Pk,
    mldsa44Pk,
    ed25519Sig,
    mldsa44Sig,
    message,
  });
}

// Test-only helpers exported so JS test files outside `scripts/hybrid-verifier/`
// do not need bare `@noble/*` module resolution across directories.
export function mldsa44KeygenForTest(seedBytes) {
  return ml_dsa44.keygen(seedBytes);
}

export function mldsa44SignForTest(messageBytes, secretKeyBytes, opts) {
  return ml_dsa44.sign(messageBytes, secretKeyBytes, opts);
}

export function verifyEd25519CofactoredForTest(sigBytes, messageBytes, pkBytes) {
  return ed25519.verify(sigBytes, messageBytes, pkBytes, { zip215: false });
}
