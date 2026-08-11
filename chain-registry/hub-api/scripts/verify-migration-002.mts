import { HubDatabase } from "../src/db/index.ts";

const path = process.env.HUB_DB_PATH;
if (!path) throw new Error("HUB_DB_PATH required");
const db = new HubDatabase(path);
db.connect();
const versions = db.handle
  .prepare("SELECT version FROM schema_migrations ORDER BY version")
  .all();
const tables = db.handle
  .prepare(
    "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'arena_%' ORDER BY name",
  )
  .all();
console.log({ versions, tables });
db.close();
