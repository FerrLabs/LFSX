#!/usr/bin/env node
import { spawnSync } from "child_process";
import { existsSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";
import { createRequire } from "module";
import { constants } from "os";

const __dirname = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);

const PLATFORMS = {
  "linux-x64": "@ferrlabs/lfsx-linux-x64",
  "linux-arm64": "@ferrlabs/lfsx-linux-arm64",
  "darwin-x64": "@ferrlabs/lfsx-darwin-x64",
  "darwin-arm64": "@ferrlabs/lfsx-darwin-arm64",
  "win32-x64": "@ferrlabs/lfsx-win32-x64",
};

function binaryPath() {
  const ext = process.platform === "win32" ? ".exe" : "";
  const pkg = PLATFORMS[`${process.platform}-${process.arch}`];

  if (pkg) {
    try {
      return require.resolve(`${pkg}/bin/lfsx${ext}`);
    } catch {
      // the optional dependency for this platform is not installed
    }
  }

  const devBuild = join(__dirname, "..", "..", "..", "target", "release", `lfsx${ext}`);
  if (existsSync(devBuild)) return devBuild;

  console.error(
    `lfsx: no binary for ${process.platform}-${process.arch}\n` +
      "Install it from https://github.com/FerrLabs/LFSX/releases"
  );
  process.exit(1);
}

const binary = binaryPath();
const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  console.error(`lfsx: could not launch ${binary}: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) {
  process.exit(128 + (constants.signals[result.signal] ?? 0));
}
process.exit(result.status ?? 1);
