#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import {
  dirname,
  isAbsolute,
  relative,
  resolve,
} from "node:path";
import { fileURLToPath } from "node:url";

import {
  computeGenesisHash,
  validateActivationGate,
  verifyChainSpecSignature,
} from "./validate-cr07-activation.mjs";

export const MATRIX_SCHEMA_VERSION = 1;
export const MATRIX_KIND = "creg-repository-capability-matrix";
export const GENERATOR_ID = "creg-me-28-capability-gate";
export const GENERATOR_VERSION = 1;

export const CR07_MANIFEST_PATH =
  "config/cr07-activation-manifest.json";
export const WEAK_SUBJECTIVITY_TEMPLATE_PATH =
  "config/weak-subjectivity/canonical-anchor.example.json";
export const REGISTRY_CONTRACT_PATH = "contracts/Registry.sol";
export const SEPOLIA_CANONICAL_SPEC_PATH =
  "testnet/chain-spec.sepolia.json";
export const SEPOLIA_CANONICAL_SIGNATURE_PATH =
  "testnet/chain-spec.sepolia.json.sig";
export const SEPOLIA_SIGNING_GUARD_PATH =
  "testnet/require-spec-signing-key.ps1";
export const SEPOLIA_PUBLICATION_MIRRORS = Object.freeze([
  Object.freeze({
    spec_path: "testnet/spec-server/chain-spec.sepolia.json",
    signature_path:
      "testnet/spec-server/chain-spec.sepolia.json.sig",
  }),
  Object.freeze({
    spec_path: "testnet/spec-server/chain-spec.json",
    signature_path: "testnet/spec-server/chain-spec.json.sig",
  }),
]);

export const CR07_GUARDED_CHAIN_SPECS = Object.freeze([
  "testnet/chain-spec.local.json",
  "testnet/chain-spec.sepolia.json",
  "testnet/spec-server/chain-spec.json",
  "testnet/spec-server/chain-spec.sepolia.json",
]);

export const SHIPPED_CHAIN_SPECS = Object.freeze([
  ...CR07_GUARDED_CHAIN_SPECS,
  "testnet/chain-spec.example.json",
  "testnet/genesis.json",
]);

export const ROLLOUT_SURFACES = Object.freeze([
  ".env.example",
  "docker-compose.light.yml",
  "docker-compose.prod.yml",
  "docker-compose.testnet.yml",
  "docker-compose.validator.yml",
  "docker-compose.yml",
  "k8s/01-configmap.yaml",
  "testnet/.env.example",
  "testnet/bootstrap/bootstrap.env.example",
  "testnet/bootstrap/docker-compose.yml",
  "testnet/docker-compose.observer-pool.yml",
  "testnet/docker-compose.validator-fleet.yml",
  "testnet/docker-compose.validator.yml",
  "testnet/sepolia-fleet.env.example",
]);

const FEATURE_FLAG_KEYS = Object.freeze([
  "cross_chain",
  "insurance",
  "ml_validation",
  "private_registries",
  "threshold_encryption",
  "wasm_sandbox",
  "zk_validation",
]);
const CHECKPOINT_ENV = "CREG_WEAK_SUBJECTIVITY_CHECKPOINT";
const CERTIFIED_COMMITTEES_ENV =
  "CREG_REQUIRE_CERTIFIED_COMMITTEES";
const MAX_INPUT_BYTES = 2 * 1024 * 1024;

export const MATRIX_INPUT_PATHS = Object.freeze(
  [
    "scripts/generate-capability-matrix.mjs",
    "scripts/validate-cr07-activation.mjs",
    CR07_MANIFEST_PATH,
    WEAK_SUBJECTIVITY_TEMPLATE_PATH,
    REGISTRY_CONTRACT_PATH,
    SEPOLIA_CANONICAL_SIGNATURE_PATH,
    SEPOLIA_SIGNING_GUARD_PATH,
    ...SEPOLIA_PUBLICATION_MIRRORS.map(
      (mirror) => mirror.signature_path,
    ),
    ...SHIPPED_CHAIN_SPECS,
    ...ROLLOUT_SURFACES,
  ].sort(),
);

function fail(message) {
  throw new Error(`capability matrix gate: ${message}`);
}

function repositoryPath(root, path) {
  if (
    typeof path !== "string" ||
    path.length === 0 ||
    isAbsolute(path)
  ) {
    fail(`invalid repository-relative path: ${String(path)}`);
  }
  const absolute = resolve(root, path);
  const fromRoot = relative(root, absolute);
  if (
    fromRoot === ".." ||
    fromRoot.startsWith("../") ||
    fromRoot.startsWith("..\\")
  ) {
    fail(`path escapes repository root: ${path}`);
  }
  return absolute;
}

function readBounded(root, path) {
  let value;
  try {
    value = readFileSync(repositoryPath(root, path));
  } catch (error) {
    fail(`cannot read required input ${path}: ${error.message}`);
  }
  if (value.length > MAX_INPUT_BYTES) {
    fail(`${path} exceeds the ${MAX_INPUT_BYTES}-byte input limit`);
  }
  return value;
}

function readText(root, path) {
  return readBounded(root, path).toString("utf8");
}

function readJson(root, path) {
  try {
    return JSON.parse(readText(root, path));
  } catch (error) {
    if (error.message.startsWith("capability matrix gate:")) {
      throw error;
    }
    fail(`${path} is not valid JSON: ${error.message}`);
  }
}

function requireObject(value, path) {
  if (
    value === null ||
    typeof value !== "object" ||
    Array.isArray(value)
  ) {
    fail(`${path} must be an object`);
  }
  return value;
}

function sortedUniqueStrings(values, path) {
  if (!Array.isArray(values)) {
    fail(`${path} must be an array`);
  }
  const result = [];
  const seen = new Set();
  for (const [index, value] of values.entries()) {
    if (
      typeof value !== "string" ||
      value.length === 0 ||
      value.trim() !== value
    ) {
      fail(`${path}[${index}] must be a canonical non-empty string`);
    }
    if (seen.has(value)) {
      fail(`${path} contains duplicate ${value}`);
    }
    seen.add(value);
    result.push(value);
  }
  return result.sort();
}

function sameStrings(actual, expected, path) {
  const left = sortedUniqueStrings(actual, path);
  const right = [...expected].sort();
  if (
    left.length !== right.length ||
    left.some((value, index) => value !== right[index])
  ) {
    fail(
      `${path} must contain exactly ${right.join(", ")}; got ${left.join(", ")}`,
    );
  }
}

function inspectChainSpec(root, path, cr07Guarded) {
  const spec = requireObject(readJson(root, path), path);
  const flags = requireObject(
    spec.feature_flags,
    `${path}.feature_flags`,
  );
  const keys = Object.keys(flags).sort();
  if (
    keys.length !== FEATURE_FLAG_KEYS.length ||
    keys.some((key, index) => key !== FEATURE_FLAG_KEYS[index])
  ) {
    fail(
      `${path}.feature_flags must contain exactly the supported capability schema; got ${keys.join(", ")}`,
    );
  }
  for (const key of FEATURE_FLAG_KEYS) {
    if (typeof flags[key] !== "boolean") {
      fail(`${path}.feature_flags.${key} must be a boolean`);
    }
  }
  if (flags.zk_validation !== false) {
    fail(`${path}.feature_flags.zk_validation must remain false`);
  }

  const consensus = requireObject(
    spec.consensus_params,
    `${path}.consensus_params`,
  );
  const hasActivationHeight = Object.hasOwn(
    consensus,
    "namespace_authority_activation_height",
  );
  const hasAuthorityPolicies = Object.hasOwn(
    consensus,
    "namespace_authority_policies",
  );
  if (
    !cr07Guarded &&
    (hasActivationHeight || hasAuthorityPolicies)
  ) {
    fail(
      `${path} contains CR-07 activation fields but is not controlled by the signed activation manifest`,
    );
  }

  return {
    path,
    chain_id:
      typeof spec.chain_id === "string" ? spec.chain_id : null,
    network:
      typeof spec.network === "string" ? spec.network : null,
    phase: typeof spec.phase === "string" ? spec.phase : null,
    zk_validation: false,
    legacy_batch_flag_representable: false,
    cr07_manifest_guarded: cr07Guarded,
  };
}

function solidityCodeOnly(source) {
  let state = "code";
  let result = "";
  for (let index = 0; index < source.length; index += 1) {
    const current = source[index];
    const next = source[index + 1];
    if (state === "code" && current === "/" && next === "/") {
      state = "line-comment";
      result += "  ";
      index += 1;
    } else if (
      state === "code" &&
      current === "/" &&
      next === "*"
    ) {
      state = "block-comment";
      result += "  ";
      index += 1;
    } else if (
      state === "code" &&
      (current === '"' || current === "'")
    ) {
      state = current === '"' ? "double-string" : "single-string";
      result += " ";
    } else if (state === "line-comment") {
      if (current === "\n") {
        state = "code";
        result += "\n";
      } else {
        result += " ";
      }
    } else if (state === "block-comment") {
      if (current === "*" && next === "/") {
        state = "code";
        result += "  ";
        index += 1;
      } else {
        result += current === "\n" ? "\n" : " ";
      }
    } else if (
      state === "double-string" ||
      state === "single-string"
    ) {
      const closing =
        (state === "double-string" && current === '"') ||
        (state === "single-string" && current === "'");
      if (current === "\\" && next !== undefined) {
        result += "  ";
        index += 1;
      } else if (closing) {
        state = "code";
        result += " ";
      } else {
        result += current === "\n" ? "\n" : " ";
      }
    } else {
      result += current;
    }
  }
  if (state !== "code" && state !== "line-comment") {
    fail(`${REGISTRY_CONTRACT_PATH} contains unterminated Solidity trivia`);
  }
  return result;
}

function stripSolidityTrivia(value) {
  return value.replace(/\s+/gu, "");
}

function solidityFunctionBody(source, functionName) {
  const declarations = [
    ...source.matchAll(
      new RegExp(
        `\\bfunction\\s+${functionName}\\s*\\(`,
        "gu",
      ),
    ),
  ];
  if (declarations.length === 0) {
    fail(`${REGISTRY_CONTRACT_PATH} is missing ${functionName}()`);
  }
  if (declarations.length !== 1) {
    fail(
      `${REGISTRY_CONTRACT_PATH} must contain exactly one ${functionName}() entrypoint`,
    );
  }
  const openingBrace = source.indexOf("{", declarations[0].index);
  if (openingBrace === -1) {
    fail(`${functionName}() has no function body`);
  }
  let depth = 0;
  for (let index = openingBrace; index < source.length; index += 1) {
    if (source[index] === "{") depth += 1;
    if (source[index] === "}") {
      depth -= 1;
      if (depth === 0) {
        return source.slice(openingBrace + 1, index);
      }
    }
  }
  fail(`${functionName}() has an unterminated function body`);
}

function inspectRetiredContractPaths(root) {
  const source = solidityCodeOnly(
    readText(root, REGISTRY_CONTRACT_PATH),
  );
  const requiredBodies = new Map([
    ["submitPackageWithZKProof", "revertZKQualificationRetired();"],
    ["verifyZKProof", "revertZKQualificationRetired();"],
    ["submitRollupBatch", "revertLegacyCheckpointingRetired();"],
  ]);
  for (const [functionName, expectedBody] of requiredBodies) {
    const body = stripSolidityTrivia(
      solidityFunctionBody(source, functionName),
    );
    if (body !== expectedBody) {
      fail(
        `${REGISTRY_CONTRACT_PATH} ${functionName}() must contain only ${expectedBody}`,
      );
    }
  }

  const setter = stripSolidityTrivia(
    solidityFunctionBody(source, "setZKValidationEnabled"),
  );
  if (
    setter !==
    "if(enabled)revertZKQualificationRetired();zkValidationEnabled=enabled;emitZKValidationToggled(enabled);"
  ) {
    fail(
      `${REGISTRY_CONTRACT_PATH} setZKValidationEnabled() no longer rejects every true transition`,
    );
  }
  const zkStateAssignments = [
    ...source.matchAll(
      /\bzkValidationEnabled\s*=\s*([^;]+);/gu,
    ),
  ].map((match) => stripSolidityTrivia(match[1]));
  if (
    zkStateAssignments.length !== 2 ||
    zkStateAssignments[0] !== "false" ||
    zkStateAssignments[1] !== "enabled"
  ) {
    fail(
      `${REGISTRY_CONTRACT_PATH} must initialize retired ZK qualification to false and expose no additional state assignment`,
    );
  }
  const legacyWrites = [
    /\blatestStateRoot\s*(?:=|\+=|-=|\+\+|--)/gu,
    /\btotalBatches\s*(?:=|\+=|-=|\+\+|--)/gu,
    /\bbatchRoots\s*\[[^\]]+\]\s*=/gu,
    /\bbatchDataRoots\s*\[[^\]]+\]\s*=/gu,
  ].flatMap((pattern) => [...source.matchAll(pattern)]);
  if (legacyWrites.length !== 0) {
    fail(
      `${REGISTRY_CONTRACT_PATH} contains a state-writing legacy batch path`,
    );
  }

  return {
    path: REGISTRY_CONTRACT_PATH,
    zk_entrypoints: [
      "submitPackageWithZKProof",
      "verifyZKProof",
    ],
    zk_enable_setter: "setZKValidationEnabled",
    legacy_batch_entrypoint: "submitRollupBatch",
    source_guard: "unconditional_revert",
    deployed_bytecode_evaluated: false,
  };
}

function unquote(value) {
  const trimmed = value.trim();
  if (
    trimmed.length >= 2 &&
    ((trimmed.startsWith('"') && trimmed.endsWith('"')) ||
      (trimmed.startsWith("'") && trimmed.endsWith("'")))
  ) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

function envAssignments(text, variable) {
  const escaped = variable.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
  const pattern = new RegExp(
    `^\\s*(#\\s*)?${escaped}\\s*=\\s*(.*?)\\s*$`,
    "u",
  );
  return text
    .split(/\r?\n/u)
    .map((line) => {
      const match = pattern.exec(line);
      if (!match) return null;
      return {
        commented: Boolean(match[1]),
        value: unquote(match[2]),
      };
    })
    .filter(Boolean);
}

function yamlAssignments(text, variable) {
  const escaped = variable.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
  const pattern = new RegExp(
    `^\\s*(?:-\\s*)?${escaped}\\s*(?::|=)\\s*(.*?)\\s*$`,
    "u",
  );
  return text
    .split(/\r?\n/u)
    .map((line) => {
      const match = pattern.exec(line);
      return match ? unquote(match[1]) : null;
    })
    .filter((value) => value !== null);
}

function inspectRolloutSurface(root, path) {
  const text = readText(root, path);
  const envStyle = path.endsWith(".example");
  if (envStyle) {
    const checkpoint = envAssignments(text, CHECKPOINT_ENV);
    const mandatory = envAssignments(text, CERTIFIED_COMMITTEES_ENV);
    if (checkpoint.length !== 1 || mandatory.length !== 1) {
      fail(
        `${path} must declare exactly one documented/default value for both staged rollout variables`,
      );
    }
    if (
      checkpoint[0].commented &&
      checkpoint[0].value !== '{"schema_version":1,...}'
    ) {
      fail(
        `${path} may document only the non-deployable checkpoint placeholder`,
      );
    }
    if (!checkpoint[0].commented && checkpoint[0].value !== "") {
      fail(`${path} must not ship an active checkpoint default`);
    }
    if (mandatory[0].value !== "false") {
      fail(
        `${path} must keep ${CERTIFIED_COMMITTEES_ENV}=false during staged rollout`,
      );
    }
    return {
      path,
      checkpoint_default: checkpoint[0].commented
        ? "unset_documented_placeholder"
        : "empty",
      certified_committees_default: mandatory[0].commented
        ? "unset_documented_false"
        : "false",
    };
  }

  const checkpoint = yamlAssignments(text, CHECKPOINT_ENV);
  const mandatory = yamlAssignments(text, CERTIFIED_COMMITTEES_ENV);
  if (checkpoint.length !== 1 || mandatory.length !== 1) {
    fail(
      `${path} must declare exactly one default for both staged rollout variables`,
    );
  }
  const allowedCheckpoint = new Set([
    "",
    `\${${CHECKPOINT_ENV}:-}`,
  ]);
  const allowedMandatory = new Set([
    "false",
    `\${${CERTIFIED_COMMITTEES_ENV}:-false}`,
  ]);
  if (!allowedCheckpoint.has(checkpoint[0])) {
    fail(`${path} must default ${CHECKPOINT_ENV} to empty`);
  }
  if (!allowedMandatory.has(mandatory[0])) {
    fail(
      `${path} must default ${CERTIFIED_COMMITTEES_ENV} to false`,
    );
  }
  return {
    path,
    checkpoint_default: "empty",
    certified_committees_default: "false",
  };
}

function inspectWeakSubjectivityTemplate(root) {
  const template = requireObject(
    readJson(root, WEAK_SUBJECTIVITY_TEMPLATE_PATH),
    WEAK_SUBJECTIVITY_TEMPLATE_PATH,
  );
  const source = requireObject(
    template.source,
    `${WEAK_SUBJECTIVITY_TEMPLATE_PATH}.source`,
  );
  const placeholders = [
    template.chain_id,
    template.network,
    template.block_hash,
    template.validator_set_hash,
    template.observed_at,
    source.operator_id,
    source.reference,
    source.evidence_sha256,
  ];
  if (
    template.schema_version !== 1 ||
    template.block_height !== 0 ||
    template.canonical_head_height !== 0 ||
    placeholders.some(
      (value) =>
        typeof value !== "string" ||
        !value.startsWith("REPLACE_WITH_"),
    )
  ) {
    fail(
      `${WEAK_SUBJECTIVITY_TEMPLATE_PATH} must remain an explicitly non-deployable placeholder`,
    );
  }
  return {
    path: WEAK_SUBJECTIVITY_TEMPLATE_PATH,
    repository_state: "non_deployable_placeholder",
    signed_checkpoint_evidence_evaluated: false,
  };
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function disclosedDevelopmentSigningKeys(root) {
  const source = readText(root, SEPOLIA_SIGNING_GUARD_PATH);
  const declaration =
    /\$disclosedDevelopmentPubkeys\s*=\s*@\(([\s\S]*?)\)/u.exec(
      source,
    );
  if (!declaration) {
    fail(
      `${SEPOLIA_SIGNING_GUARD_PATH} must retain the disclosed-development-key denylist`,
    );
  }
  const keys = [
    ...declaration[1].matchAll(
      /["']((?:0x)?[0-9a-f]{64})["']/gu,
    ),
  ].map((match) => match[1].replace(/^0x/u, ""));
  if (keys.length === 0) {
    fail(
      `${SEPOLIA_SIGNING_GUARD_PATH} disclosed-development-key denylist must not be empty`,
    );
  }
  return new Set(
    sortedUniqueStrings(
      keys,
      `${SEPOLIA_SIGNING_GUARD_PATH} disclosed development keys`,
    ),
  );
}

function inspectSepoliaPublication(root, activationManifest) {
  const canonicalBytes = readBounded(
    root,
    SEPOLIA_CANONICAL_SPEC_PATH,
  );
  const canonical = requireObject(
    readJson(root, SEPOLIA_CANONICAL_SPEC_PATH),
    SEPOLIA_CANONICAL_SPEC_PATH,
  );
  const signatureBytes = readBounded(
    root,
    SEPOLIA_CANONICAL_SIGNATURE_PATH,
  );
  const signatureHex = signatureBytes.toString("utf8").trim();
  const signatureFormatValid = /^(?:0x)?[0-9a-f]{128}$/u.test(
    signatureHex,
  );

  let computedGenesisHash = null;
  try {
    computedGenesisHash = computeGenesisHash(canonical);
  } catch {
    // A malformed signed payload is a release blocker, not a reason for the
    // ordinary truth-reporting mode to hide the rest of the matrix.
  }
  const genesisHashValid =
    typeof canonical.genesis_hash === "string" &&
    computedGenesisHash !== null &&
    canonical.genesis_hash === computedGenesisHash;

  let signatureCryptographicallyValid = false;
  if (signatureFormatValid) {
    try {
      verifyChainSpecSignature(
        canonical,
        signatureHex.replace(/^0x/u, ""),
        SEPOLIA_CANONICAL_SPEC_PATH,
      );
      signatureCryptographicallyValid = true;
    } catch {
      // As above, report the blocker in ordinary mode. Enforcement happens in
      // the explicit release-readiness mode.
    }
  }

  const mirrors = SEPOLIA_PUBLICATION_MIRRORS.map((mirror) => {
    const mirrorSpecBytes = readBounded(root, mirror.spec_path);
    const mirrorSignatureBytes = readBounded(
      root,
      mirror.signature_path,
    );
    const specBytesEqual = mirrorSpecBytes.equals(canonicalBytes);
    const signatureBytesEqual =
      mirrorSignatureBytes.equals(signatureBytes);
    return {
      spec_path: mirror.spec_path,
      signature_path: mirror.signature_path,
      spec_sha256: sha256(mirrorSpecBytes),
      signature_sha256: sha256(mirrorSignatureBytes),
      spec_byte_length: mirrorSpecBytes.length,
      signature_byte_length: mirrorSignatureBytes.length,
      exact_spec_bytes: specBytesEqual,
      exact_signature_bytes: signatureBytesEqual,
      exact_publication_pair:
        specBytesEqual && signatureBytesEqual,
      status:
        specBytesEqual && signatureBytesEqual
          ? "valid"
          : "blocked",
    };
  });
  const mirrorsExact = mirrors.every(
    (mirror) => mirror.exact_publication_pair,
  );

  const signing = requireObject(
    canonical.signing,
    `${SEPOLIA_CANONICAL_SPEC_PATH}.signing`,
  );
  const configuredPubkey =
    typeof signing.signing_key_pubkey_hex === "string"
      ? signing.signing_key_pubkey_hex.replace(/^0x/u, "")
      : "";
  const disclosedKeys = disclosedDevelopmentSigningKeys(root);
  const disallowedKeys = new Set(
    sortedUniqueStrings(
      activationManifest.disallowed_signing_pubkeys,
      `${CR07_MANIFEST_PATH}.disallowed_signing_pubkeys`,
    ).map((key) => key.replace(/^0x/u, "")),
  );
  const repositoryDisclosed =
    disclosedKeys.has(configuredPubkey);
  const disallowed = disallowedKeys.has(configuredPubkey);
  const signingKeyAcceptable =
    configuredPubkey.length === 64 &&
    !repositoryDisclosed &&
    !disallowed;

  const blockerFacts = [
    {
      id: "sepolia_canonical_integrity_invalid",
      blocked:
        !genesisHashValid ||
        !signatureCryptographicallyValid,
    },
    {
      id: "sepolia_publication_mirrors_diverge",
      blocked: !mirrorsExact,
    },
    {
      id: "sepolia_signing_key_unacceptable",
      blocked: !signingKeyAcceptable,
    },
  ];
  const blockers = blockerFacts
    .filter((fact) => fact.blocked)
    .map((fact) => fact.id);

  return {
    status: blockers.length === 0 ? "ready" : "blocked",
    release_ready: blockers.length === 0,
    readiness_scope:
      "signed_sepolia_repository_publication_only",
    blockers,
    canonical: {
      spec_path: SEPOLIA_CANONICAL_SPEC_PATH,
      signature_path: SEPOLIA_CANONICAL_SIGNATURE_PATH,
      declared_genesis_hash:
        typeof canonical.genesis_hash === "string"
          ? canonical.genesis_hash
          : null,
      computed_genesis_hash: computedGenesisHash,
      genesis_hash_valid: genesisHashValid,
      spec_sha256: sha256(canonicalBytes),
      signature_sha256: sha256(signatureBytes),
      spec_byte_length: canonicalBytes.length,
      signature_byte_length: signatureBytes.length,
      signature_format_valid: signatureFormatValid,
      signature_cryptographically_valid:
        signatureCryptographicallyValid,
      status:
        genesisHashValid && signatureCryptographicallyValid
          ? "valid"
          : "blocked",
    },
    mirrors,
    mirrors_exact: mirrorsExact,
    signing_key: {
      configured_pubkey_hex: configuredPubkey || null,
      repository_disclosed_development_key:
        repositoryDisclosed,
      disclosure_source: SEPOLIA_SIGNING_GUARD_PATH,
      disallowed_by_activation_manifest: disallowed,
      disallow_source: CR07_MANIFEST_PATH,
      acceptable_for_publication: signingKeyAcceptable,
      status: signingKeyAcceptable ? "valid" : "blocked",
    },
  };
}

function matrixInputPaths(activationManifest) {
  const paths = [...MATRIX_INPUT_PATHS];
  if (activationManifest.status === "signed") {
    const activation = requireObject(
      activationManifest.activation,
      `${CR07_MANIFEST_PATH}.activation`,
    );
    paths.push(activation.signature_path);
    if (Array.isArray(activation.mirrors)) {
      for (const mirror of activation.mirrors) {
        if (mirror && typeof mirror === "object") {
          paths.push(mirror.signature_path);
        }
      }
    }
  }
  return sortedUniqueStrings(paths, "capability matrix input paths");
}

function inputDigests(root, paths) {
  return paths.map((path) => ({
    path,
    sha256: sha256(readBounded(root, path)),
  }));
}

export function buildCapabilityMatrix({
  root = process.cwd(),
} = {}) {
  root = resolve(root);

  const activationManifest = requireObject(
    readJson(root, CR07_MANIFEST_PATH),
    CR07_MANIFEST_PATH,
  );
  sameStrings(
    activationManifest.guarded_chain_specs,
    CR07_GUARDED_CHAIN_SPECS,
    `${CR07_MANIFEST_PATH}.guarded_chain_specs`,
  );

  let cr07;
  try {
    cr07 = validateActivationGate({
      root,
      manifestPath: CR07_MANIFEST_PATH,
    });
  } catch (error) {
    fail(
      `existing CR-07 activation validator rejected repository state: ${error.message}`,
    );
  }

  const guarded = new Set(CR07_GUARDED_CHAIN_SPECS);
  const chainSpecs = SHIPPED_CHAIN_SPECS.map((path) =>
    inspectChainSpec(root, path, guarded.has(path)),
  );
  const contract = inspectRetiredContractPaths(root);
  const weakSubjectivity = inspectWeakSubjectivityTemplate(root);
  const rollout = ROLLOUT_SURFACES.map((path) =>
    inspectRolloutSurface(root, path),
  );
  const sepoliaPublication = inspectSepoliaPublication(
    root,
    activationManifest,
  );

  return {
    schema_version: MATRIX_SCHEMA_VERSION,
    kind: MATRIX_KIND,
    generator: {
      id: GENERATOR_ID,
      version: GENERATOR_VERSION,
    },
    scope: {
      assertion: "repository_configuration_only",
      deployed_state: "not_evaluated",
      signed_checkpoint_publication: "not_evaluated",
      release_readiness:
        "signed_sepolia_repository_publication_only",
      complete_system_release_readiness: "not_evaluated",
    },
    release_readiness: {
      status: sepoliaPublication.status,
      ready: sepoliaPublication.release_ready,
      scope: sepoliaPublication.readiness_scope,
      blockers: sepoliaPublication.blockers,
      complete_system_release_readiness: "not_evaluated",
    },
    inputs: inputDigests(
      root,
      matrixInputPaths(activationManifest),
    ),
    chain_specs: chainSpecs,
    capabilities: [
      {
        id: "zk_package_qualification",
        repository_state: "retired",
        chain_spec_state: "disabled",
        source_guard: "unconditional_revert",
        deployed_state: "not_evaluated",
      },
      {
        id: "legacy_batch_checkpointing",
        repository_state: "retired",
        chain_spec_representation: "absent",
        source_guard: "unconditional_revert",
        deployed_state: "not_evaluated",
      },
      {
        id: "cr07_authoritative_namespace",
        repository_state:
          cr07.status === "disabled"
            ? "disabled"
            : "valid_signed_activation",
        activation_manifest: CR07_MANIFEST_PATH,
        guarded_chain_specs: cr07.guardedSpecs,
        activation_spec: cr07.activationSpec,
        genesis_hash: cr07.genesisHash ?? null,
        canonical_spec_sha256:
          cr07.canonicalSpecSha256 ?? null,
        activation_payload_sha256:
          cr07.activationPayloadSha256 ?? null,
        deployed_state: "not_evaluated",
      },
      {
        id: "weak_subjectivity_checkpoint",
        repository_state: "staged_unconfigured_defaults",
        template_state: weakSubjectivity.repository_state,
        rollout_surfaces: rollout.length,
        published_checkpoint: "not_evaluated",
        deployed_state: "not_evaluated",
      },
      {
        id: "certified_committee_mandatory_mode",
        repository_state: "staged_compatibility_defaults",
        mandatory_default: false,
        rollout_surfaces: rollout.length,
        deployed_state: "not_evaluated",
      },
      {
        id: "signed_sepolia_spec_publication",
        repository_state: sepoliaPublication.status,
        release_ready: sepoliaPublication.release_ready,
        blockers: sepoliaPublication.blockers,
        deployed_state: "not_evaluated",
      },
    ],
    sepolia_publication_integrity: sepoliaPublication,
    retirement_contract: contract,
    weak_subjectivity_template: weakSubjectivity,
    rollout_defaults: rollout,
  };
}

export function serializeCapabilityMatrix(matrix) {
  return `${JSON.stringify(matrix, null, 2)}\n`;
}

export function assertReleaseReady(matrix) {
  const readiness = requireObject(
    matrix.release_readiness,
    "matrix.release_readiness",
  );
  const publication = requireObject(
    matrix.sepolia_publication_integrity,
    "matrix.sepolia_publication_integrity",
  );
  if (
    readiness.ready !== publication.release_ready ||
    readiness.status !== publication.status ||
    readiness.scope !== publication.readiness_scope ||
    JSON.stringify(readiness.blockers) !==
      JSON.stringify(publication.blockers)
  ) {
    fail(
      "release readiness summary is inconsistent with Sepolia publication integrity facts",
    );
  }
  if (
    readiness.ready !== true ||
    readiness.status !== "ready" ||
    !Array.isArray(readiness.blockers) ||
    readiness.blockers.length !== 0
  ) {
    const blockers = Array.isArray(readiness.blockers)
      ? readiness.blockers.join(", ")
      : "malformed_readiness_result";
    fail(
      `release readiness is blocked for ${String(readiness.scope)}: ${blockers}`,
    );
  }
  return matrix;
}

function usage() {
  console.log(`Usage:
  node scripts/generate-capability-matrix.mjs
  node scripts/generate-capability-matrix.mjs --output FILE
  node scripts/generate-capability-matrix.mjs --check FILE
  [all forms accept --root PATH and --require-release-ready]

The report proves only the checked-out repository configuration. It does not
assert that a chain spec, contract bytecode, checkpoint, or activation is
deployed. --require-release-ready separately fails unless the canonical
Sepolia spec, both publication mirrors, and signing-key controls pass; it is
not a complete system-release certification.`);
}

function parseArguments(argv) {
  const options = new Map();
  const flags = new Set();
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--help" || argument === "-h") {
      return { help: true, options, flags };
    }
    if (argument === "--require-release-ready") {
      if (flags.has(argument)) {
        fail(`${argument} was specified more than once`);
      }
      flags.add(argument);
      continue;
    }
    if (!["--root", "--output", "--check"].includes(argument)) {
      fail(`unknown argument ${argument}`);
    }
    if (options.has(argument)) {
      fail(`${argument} was specified more than once`);
    }
    const value = argv[index + 1];
    if (!value || value.startsWith("--")) {
      fail(`${argument} requires a value`);
    }
    options.set(argument, value);
    index += 1;
  }
  if (options.has("--output") && options.has("--check")) {
    fail("--output and --check are mutually exclusive");
  }
  return { help: false, options, flags };
}

export function runCli(argv) {
  const { help, options, flags } = parseArguments(argv);
  if (help) {
    usage();
    return;
  }
  const root = options.get("--root") ?? process.cwd();
  const matrix = buildCapabilityMatrix({ root });
  const serialized = serializeCapabilityMatrix(matrix);
  const checkPath = options.get("--check");
  if (checkPath) {
    let checkedIn;
    try {
      checkedIn = readFileSync(
        repositoryPath(resolve(root), checkPath),
        "utf8",
      );
    } catch (error) {
      fail(`cannot read checked artifact ${checkPath}: ${error.message}`);
    }
    if (checkedIn !== serialized) {
      fail(
        `${checkPath} is stale; regenerate it with --output ${checkPath}`,
      );
    }
  }
  if (flags.has("--require-release-ready")) {
    assertReleaseReady(matrix);
  }
  if (checkPath) {
    console.log(
      JSON.stringify({
        status: "valid",
        artifact: checkPath,
        sha256: createHash("sha256")
          .update(serialized)
          .digest("hex"),
      }),
    );
    return;
  }

  const outputPath = options.get("--output");
  if (outputPath) {
    const absolute = repositoryPath(resolve(root), outputPath);
    mkdirSync(dirname(absolute), { recursive: true });
    writeFileSync(absolute, serialized, "utf8");
    console.log(
      JSON.stringify({
        status: "written",
        artifact: outputPath,
        sha256: createHash("sha256")
          .update(serialized)
          .digest("hex"),
      }),
    );
    return;
  }
  process.stdout.write(serialized);
}

const invokedPath = process.argv[1]
  ? resolve(process.argv[1])
  : "";
if (invokedPath === resolve(fileURLToPath(import.meta.url))) {
  try {
    runCli(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
