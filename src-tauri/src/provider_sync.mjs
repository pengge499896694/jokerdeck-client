import path from "node:path";
import { pathToFileURL } from "node:url";
const [servicePath, action, codexHome, backupId] = process.argv.slice(2);
try {
  const service = await import(pathToFileURL(servicePath));
  let result;
  if (action === "status") {
    result = await service.getStatus({ codexHome });
    const { listBackups } = await import(pathToFileURL(path.join(path.dirname(servicePath), "backup.js")));
    result.backups = (await listBackups(codexHome)).backups;
    result.rolloutCounts = Object.entries(result.rolloutCounts).flatMap(([scope, providers]) => Object.entries(providers).map(([provider, count]) => [scope + ":" + provider, count]));
    result.rolloutCounts = Object.fromEntries(result.rolloutCounts);
  } else if (action === "sync") {
    // Unlike the upstream CLI, leave all historical model names untouched.
    result = await service.runSync({ codexHome, keepCount: 3, model: null });
    result.sessionFilesUpdated = result.changedSessionFiles;
  } else if (action === "restore") {
    if (!backupId || path.basename(backupId) !== backupId || !/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(backupId)) throw new Error("备份标识无效");
    result = await service.runRestore({ codexHome, backupDir: path.join(codexHome, "backups_state/provider-sync", backupId), restoreConfig: false });
  } else throw new Error("未知操作");
  console.log(JSON.stringify(result));
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
