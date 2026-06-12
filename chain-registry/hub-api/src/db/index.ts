import Database from "better-sqlite3";
import { existsSync, mkdirSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";

export type DbState = "not_configured" | "ready" | "error";

export class HubDatabase {
  readonly path: string;
  private db: Database.Database | null = null;
  private state: DbState = "not_configured";
  private errorMessage: string | undefined;

  constructor(dbPath: string) {
    this.path = dbPath;
  }

  getState(): DbState {
    return this.state;
  }

  getErrorMessage(): string | undefined {
    return this.errorMessage;
  }

  connect(): void {
    if (this.db) return;

    try {
      mkdirSync(dirname(this.path), { recursive: true });
      this.db = new Database(this.path);
      this.db.pragma("journal_mode = WAL");
      this.db.pragma("foreign_keys = ON");
      this.runMigrations();
      this.state = "ready";
    } catch (error) {
      this.state = "error";
      this.errorMessage =
        error instanceof Error ? error.message : "Database initialization failed";
      throw error;
    }
  }

  get handle(): Database.Database {
    if (!this.db) {
      throw new Error("Database not connected");
    }
    return this.db;
  }

  close(): void {
    this.db?.close();
    this.db = null;
  }

  private runMigrations(): void {
    const db = this.handle;
    db.exec(`
      CREATE TABLE IF NOT EXISTS schema_migrations (
        version INTEGER PRIMARY KEY NOT NULL,
        applied_at TEXT NOT NULL
      );
    `);

    const migrationsDir =
      process.env.HUB_MIGRATIONS_DIR ?? join(process.cwd(), "migrations");
    if (!existsSync(migrationsDir)) {
      return;
    }

    const files = readdirSync(migrationsDir)
      .filter((name) => /^\d+_.+\.sql$/.test(name))
      .sort();

    const applied = new Set(
      db
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .all()
        .map((row) => (row as { version: number }).version),
    );

    const applyMigration = db.transaction((version: number, sql: string) => {
      db.exec(sql);
      db.prepare(
        "INSERT INTO schema_migrations (version, applied_at) VALUES (?, ?)",
      ).run(version, new Date().toISOString());
    });

    for (const file of files) {
      const version = Number(file.split("_")[0]);
      if (!Number.isFinite(version) || applied.has(version)) continue;
      const sql = readFileSync(join(migrationsDir, file), "utf8");
      applyMigration(version, sql);
    }
  }
}

export function normalizeAddress(address: string): string {
  return address.trim().toLowerCase();
}
