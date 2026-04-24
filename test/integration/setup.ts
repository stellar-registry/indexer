import * as fs from 'fs';
import * as path from 'path';
import { Pool } from 'pg';

const REPO_ROOT = path.resolve(__dirname, '..', '..');

export const PG_URL =
  process.env.TEST_PG_URL ?? 'postgres://test:test@localhost:5433/registry_test';

let sharedPool: Pool | undefined;

export function pgPool(): Pool {
  if (!sharedPool) {
    sharedPool = new Pool({ connectionString: PG_URL });
  }
  return sharedPool;
}

export async function closePool(): Promise<void> {
  if (sharedPool) {
    await sharedPool.end();
    sharedPool = undefined;
  }
}

export async function applySql(relPath: string): Promise<void> {
  const abs = path.join(REPO_ROOT, relPath);
  const sql = fs.readFileSync(abs, 'utf8');
  await pgPool().query(sql);
}

export async function truncateAll(tables: string[]): Promise<void> {
  if (tables.length === 0) return;
  await pgPool().query(`TRUNCATE ${tables.join(', ')} RESTART IDENTITY CASCADE`);
}

export async function waitForPg(timeoutMs = 30000): Promise<void> {
  const start = Date.now();
  const probe = new Pool({ connectionString: PG_URL });
  try {
    while (true) {
      try {
        await probe.query('SELECT 1');
        return;
      } catch (err) {
        if (Date.now() - start > timeoutMs) throw err;
        await new Promise((r) => setTimeout(r, 500));
      }
    }
  } finally {
    await probe.end();
  }
}
