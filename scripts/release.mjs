import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = new URL("../", import.meta.url);
const readJson = (path) => JSON.parse(readFileSync(new URL(path, root), "utf8"));
const [command, tag, releaseDir] = process.argv.slice(2);
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
  if (!releaseDir || !repository || !/^[\w.-]+\/[\w.-]+$/.test(repository)) {
    console.error("Manifest requires a release directory and GITHUB_REPOSITORY");
    process.exit(1);
  }
  const assets = {
    "windows-x86_64": `jokerdeck_${tag}_windows-x86_64.exe`,
    "darwin-x86_64": `jokerdeck_${tag}_darwin-x86_64.dmg`,
    "darwin-aarch64": `jokerdeck_${tag}_darwin-aarch64.dmg`,
  };
  for (const filename of Object.values(assets)) {
    if (!existsSync(join(releaseDir, filename))) {
      console.error(`Missing release asset: ${filename}`);
      process.exit(1);
    }
  }
  const platforms = Object.fromEntries(
    Object.entries(assets).map(([platform, filename]) => [
      platform,
      `https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(filename)}`,
    ]),
  );
  writeFileSync(new URL("latest.json", root), `${JSON.stringify({
    version,
    notes: readFileSync(new URL("RELEASE_NOTES.md", root), "utf8").trim(),
    platforms,
  }, null, 2)}\n`);
  console.log("Created latest.json");
} else {
  console.error("Usage: node scripts/release.mjs verify|manifest vX.Y.Z [installer]");
  process.exit(1);
}
