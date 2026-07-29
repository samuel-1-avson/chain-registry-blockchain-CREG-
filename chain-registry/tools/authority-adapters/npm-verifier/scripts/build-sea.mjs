import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { builtinModules } from "node:module";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { build } from "esbuild";
import postject from "postject";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const buildDirectory = join(packageRoot, "build");
const distDirectory = join(packageRoot, "dist");
const bundlePath = join(buildDirectory, "sidecar.cjs");
const blobPath = join(buildDirectory, "sea-prep.blob");
const configPath = join(buildDirectory, "sea-config.json");
const executableName =
  process.platform === "win32"
    ? "creg-npm-authority-verifier.exe"
    : "creg-npm-authority-verifier";
const executablePath = join(distDirectory, executableName);
const allowedBuiltins = new Set([
  ...builtinModules,
  ...builtinModules.map((name) => `node:${name}`),
]);

function assertContained(path, expectedParent) {
  const child = resolve(path);
  const parent = `${resolve(expectedParent)}${sep}`;
  if (!child.startsWith(parent)) {
    throw new Error(`refusing to mutate path outside ${expectedParent}`);
  }
}

for (const directory of [buildDirectory, distDirectory]) {
  assertContained(directory, packageRoot);
  rmSync(directory, { recursive: true, force: true });
  mkdirSync(directory, { recursive: true });
}

const buildResult = await build({
  entryPoints: [join(packageRoot, "src", "sea-entry.mjs")],
  bundle: true,
  format: "cjs",
  legalComments: "none",
  metafile: true,
  outfile: bundlePath,
  platform: "node",
  sourcemap: false,
  target: `node${process.versions.node.split(".")[0]}`,
});
for (const output of Object.values(buildResult.metafile.outputs)) {
  for (const imported of output.imports) {
    if (imported.external && !allowedBuiltins.has(imported.path)) {
      throw new Error(`SEA bundle retained external dependency ${imported.path}`);
    }
  }
}

writeFileSync(
  configPath,
  `${JSON.stringify(
    {
      main: bundlePath,
      output: blobPath,
      disableExperimentalSEAWarning: true,
      useSnapshot: false,
      useCodeCache: false,
      execArgv: [],
      execArgvExtension: "none",
    },
    null,
    2,
  )}\n`,
  { encoding: "utf8", mode: 0o444 },
);
const prepare = spawnSync(
  process.execPath,
  ["--experimental-sea-config", configPath],
  { encoding: "utf8" },
);
if (prepare.status !== 0) {
  throw new Error(
    `Node SEA preparation failed: ${prepare.stderr || prepare.stdout}`,
  );
}

copyFileSync(process.execPath, executablePath);
if (process.platform === "darwin") {
  const unsign = spawnSync("codesign", ["--remove-signature", executablePath], {
    encoding: "utf8",
  });
  if (unsign.status !== 0) {
    throw new Error(`codesign removal failed: ${unsign.stderr || unsign.stdout}`);
  }
}
await postject.inject(
  executablePath,
  "NODE_SEA_BLOB",
  readFileSync(blobPath),
  {
    machoSegmentName: "NODE_SEA",
    sentinelFuse: "NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2",
  },
);
if (process.platform === "darwin") {
  const sign = spawnSync("codesign", ["--sign", "-", executablePath], {
    encoding: "utf8",
  });
  if (sign.status !== 0) {
    throw new Error(`codesign failed: ${sign.stderr || sign.stdout}`);
  }
}
if (process.platform !== "win32") {
  chmodSync(executablePath, 0o555);
}

const smoke = spawnSync(executablePath, [], {
  cwd: distDirectory,
  input: "{}",
  encoding: "utf8",
  env: { CREG_AUTHORITY_ADAPTER_PROTOCOL: "2" },
});
if (
  smoke.status === 0 ||
  smoke.stdout !== "" ||
  !smoke.stderr.includes("npm authority verification rejected")
) {
  throw new Error("built SEA failed its no-stdout rejection smoke test");
}
const executableBytes = readFileSync(executablePath);
const executableSha256 = createHash("sha256")
  .update(executableBytes)
  .digest("hex");
process.stdout.write(
  `${relative(packageRoot, executablePath)}\nsha256:${executableSha256}\n`,
);
