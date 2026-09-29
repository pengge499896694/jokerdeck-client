import { execFileSync } from "node:child_process";
import { basename } from "node:path";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = new URL("../", import.meta.url);
const readJson = (path) => JSON.parse(readFileSync(new URL(path, root), "utf8"));
const [command, tag, installer] = process.argv.slice(2);
const version = tag?.match(/^v(\d+\.\d+\.\d+)$/)?.[1];

if (!version) {
  console.error("Expected a release tag such as v0.1.1");
  process.exit(1);
}

if (command === "verify") {
  const packageVersion = readJson("package.json").version;
  const tauriVersion = readJson("src-tauri/tauri.conf.json").version;
  const metadata = JSON.parse(execFileSync("cargo", [
    "metadata", "--no-deps", "--format-version", "1",
    "--manifest-path", fileURLToPath(new URL("src-tauri/Cargo.toml", root)),
  ], { encoding: "utf8" }));
  const rustVersion = metadata.packages.find((item) => item.name === "jokerdeck-desktop")?.version;

  if ([packageVersion, tauriVersion, rustVersion].some((value) => value !== version)) {
    console.error(`Version mismatch: tag=${version}, package=${packageVersion}, tauri=${tauriVersion}, rust=${rustVersion}`);
    process.exit(1);
  }
  console.log(`Release version verified: ${version}`);
} else if (command === "manifest") {
  const repository = process.env.GITHUB_REPOSITORY;
  if (!installer || !repository || !/^[\w.-]+\/[\w.-]+$/.test(repository)) {
    console.error("Manifest requires an installer path and GITHUB_REPOSITORY");
    process.exit(1);
  }
  const url = `https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(basename(installer))}`;
  writeFileSync(new URL("latest.json", root), `${JSON.stringify({
    version,
    platforms: { "windows-x86_64": url },
  }, null, 2)}\n`);
  console.log("Created latest.json");
} else {
  console.error("Usage: node scripts/release.mjs verify|manifest vX.Y.Z [installer]");
  process.exit(1);
}
