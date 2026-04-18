/**
 * Postgres replay of the v4 pipeline's inline SQL transforms.
 *
 * Caveat: Goldsky's Turbo runtime is Apache Flink (Calcite SQL), not Postgres.
 * These tests are a sanity check, not a correctness proof. `JSON_VALUE` path
 * semantics, `CAST` coercion rules, and return-type behavior differ between
 * Flink and Postgres 17. Tests here will catch regressions in the portable
 * subset (which event symbols to filter, which map keys to extract, WHERE
 * clauses) but cannot catch Flink-specific dialect edge cases. The load-bearing
 * assertion is `transform_3_subregistry_events` filtering by the root contract
 * id — flipping that id in `registry-turbo-v4.yaml` will make this suite fail.
 */

import * as fs from 'fs';
import * as path from 'path';
import * as yaml from 'js-yaml';
import { pgPool, closePool } from './setup';
import { loadAllFixtures, Transform2Row } from '../fixtures/loader';

const REPO_ROOT = path.resolve(__dirname, '..', '..');
const PIPELINE_PATH = path.join(REPO_ROOT, 'registry-turbo-v4.yaml');
const NEW_ROOT = 'CBNBQND6EMYTTRTCUWUJ3VIKF7RUUISK5T4GAKTXRVIQRHGP4XQY4ID7';

interface PipelineDoc {
  transforms: Record<string, { sql: string; type: string; primary_key?: string }>;
}

function loadTransforms(): PipelineDoc['transforms'] {
  const doc = yaml.load(fs.readFileSync(PIPELINE_PATH, 'utf8')) as PipelineDoc;
  return doc.transforms;
}

async function seedTransform2(rows: Transform2Row[]): Promise<void> {
  const pool = pgPool();
  await pool.query(`
    DROP TABLE IF EXISTS transform_2_events_with_command_name;
    CREATE TABLE transform_2_events_with_command_name (
      id TEXT PRIMARY KEY,
      transaction_hash TEXT,
      ledger_sequence BIGINT,
      created_at TIMESTAMP,
      command TEXT,
      channel TEXT,
      emitter_contract_id TEXT,
      data JSONB,
      topics JSONB
    );
  `);
  for (const row of rows) {
    await pool.query(
      `INSERT INTO transform_2_events_with_command_name
       (id, transaction_hash, ledger_sequence, created_at, command, channel, emitter_contract_id, data, topics)
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb, $9::jsonb)`,
      [
        row.id,
        row.transaction_hash,
        row.ledger_sequence,
        row.created_at,
        row.command,
        row.channel,
        row.emitter_contract_id,
        row.data,
        row.topics,
      ],
    );
  }
}

describe('v4 pipeline SQL transforms replayed on Postgres', () => {
  let transforms: PipelineDoc['transforms'];

  beforeAll(async () => {
    transforms = loadTransforms();
    await seedTransform2(loadAllFixtures());
  });

  afterAll(async () => {
    await closePool();
  });

  test('transform_3_deploy_events extracts fields regardless of map order', async () => {
    const result = await pgPool().query(transforms.transform_3_deploy_events.sql);
    expect(result.rows).toHaveLength(2);
    const byName = Object.fromEntries(result.rows.map((r: any) => [r.wasm_name, r]));
    expect(byName['my_contract']).toMatchObject({
      contract_id: 'CDEPLOYED00000000000000000000000000000000000000000000',
      deployer: 'GDEPLOYER0000000000000000000000000000000000000000000',
      wasm_version: '1.0.0',
    });
    expect(byName['other_contract']).toMatchObject({
      contract_id: 'CDEPLOYED20000000000000000000000000000000000000000000',
      wasm_version: '2.0.0',
    });
  });

  test('transform_3_publish_events extracts fields regardless of map order', async () => {
    const result = await pgPool().query(transforms.transform_3_publish_events.sql);
    expect(result.rows).toHaveLength(2);
    const byHash = Object.fromEntries(result.rows.map((r: any) => [r.wasm_hash, r]));
    expect(byHash['deadbeef']).toMatchObject({
      author: 'GAUTHOR00000000000000000000000000000000000000000000000',
      wasm_version: '0.1.0',
      wasm_name: 'published_wasm',
    });
    expect(byHash['cafebabe']).toMatchObject({
      wasm_version: '0.2.0',
      wasm_name: 'published_wasm_v2',
    });
  });

  test('transform_3_register_events coerces sac bool correctly', async () => {
    const result = await pgPool().query(transforms.transform_3_register_events.sql);
    expect(result.rows).toHaveLength(2);
    const byName = Object.fromEntries(result.rows.map((r: any) => [r.contract_name, r]));
    expect(byName['token_a'].sac).toBe(true);
    expect(byName['token_b'].sac).toBe(false);
  });

  test('transform_3_rename extracts old and new names', async () => {
    const result = await pgPool().query(transforms.transform_3_rename.sql);
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0]).toMatchObject({
      old_name: 'old_token',
      new_name: 'new_token',
    });
  });

  test('transform_3_update_address extracts contract_name and new_address', async () => {
    const result = await pgPool().query(transforms.transform_3_update_address.sql);
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0]).toMatchObject({
      contract_name: 'my_token',
      new_address: 'CNEWADDRESS0000000000000000000000000000000000000000000',
    });
  });

  test('transform_3_update_owner extracts contract_name and new_owner', async () => {
    const result = await pgPool().query(transforms.transform_3_update_owner.sql);
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0]).toMatchObject({
      contract_name: 'my_token',
      new_owner: 'GNEWOWNER0000000000000000000000000000000000000000000',
    });
  });

  test('transform_3_subregistry_events only emits rows from the current root emitter', async () => {
    const sql = transforms.transform_3_subregistry_events.sql;
    expect(sql).toContain(NEW_ROOT);
    const result = await pgPool().query(sql);
    expect(result.rows).toHaveLength(2);
    const channels = new Set(result.rows.map((r: any) => r.channel));
    expect(channels).toEqual(new Set(['soroswap', 'aquarius']));
    const ids = new Set(result.rows.map((r: any) => r.contract_id));
    expect(ids).toEqual(
      new Set([
        'CSUBREG1000000000000000000000000000000000000000000000',
        'CSUBREG2000000000000000000000000000000000000000000000',
      ]),
    );
  });
});
