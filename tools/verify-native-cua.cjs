// Verify the actual patched application's feature resolver without launching its GUI.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const source = fs.readFileSync(process.argv[2], "utf8");
const resolver = /function (Er|ci)\(e,\{buildFlavor:/.exec(source);
const start = resolver?.index ?? -1;
const end = source.indexOf("function ", start + 9);
assert.ok(start >= 0 && end > start, "Native feature resolver not found");
const context = vm.createContext({
  a: { a: { Dev: "dev", resolve: () => "prod" } },
  S: { default: { env: {}, platform: "win32" } },
  rr: () => false,
  d: { t: { Dev: "dev", resolve: () => "prod" } },
  P: { default: { env: {}, platform: "win32" } },
  Or: () => false,
  ete: () => { throw new Error("Production adapter must not use dev-only overrides"); },
  Dr: () => { throw new Error("Production adapter must not use dev-only overrides"); },
});
vm.runInContext(source.slice(start, end), context);
const resolve = context[resolver[1]];
const original = { computerUse: false, computerUseNodeRepl: false,
  externalBrowserUse: false, inAppBrowserUse: false,
  codexLocalAccess: false, workCloudAccess: false, workLocalAccess: true };
for (const platform of ["win32", "darwin"]) {
  const disabled = resolve(original, { buildFlavor: "prod", env: {}, platform });
  assert.equal(disabled.computerUse, false);
  assert.equal(disabled.externalBrowserUse, false);
  const enabled = resolve(original, { buildFlavor: "prod", env: { JOKERDECK_ENABLE_NATIVE_CUA: "1" }, platform });
  for (const field of ["computerUse", "computerUseNodeRepl", "externalBrowserUse", "externalBrowserUseAllowed", "inAppBrowserUse", "inAppBrowserUseAllowed", "browserPane"]) {
    assert.equal(enabled[field], true, `${platform}: ${field}`);
  }
  assert.equal(enabled.codexLocalAccess, false);
  assert.equal(enabled.workCloudAccess, false);
  assert.equal(enabled.workLocalAccess, true);
}
const unsupported = resolve(original, { buildFlavor: "prod", env: { JOKERDECK_ENABLE_NATIVE_CUA: "1" }, platform: "linux" });
assert.equal(unsupported.computerUse, false);
assert.equal(original.computerUse, false, "Input snapshot must not be mutated");
console.log("Native CUA resolver passed: Windows/macOS opt-in, disabled state, account access preserved");
