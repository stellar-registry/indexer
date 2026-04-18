/**
 * Drift detection against real Soroban testnet events.
 *
 * Refresh fixtures with `npm run fixtures:refresh` — they live in
 * test/fixtures/soroban-events-real/ and are decoded directly from the
 * contract's event stream via Soroban RPC. Unlike the synthetic suite,
 * these assertions are structural: every row that feeds a transform must
 * have its extracted columns populated. If Goldsky's event JSON shape ever
 * diverges from what the pipeline's JSON_VALUE paths expect, the extracted
 * columns will be NULL and these tests fail — surfacing drift before deploy.
 *
 * If the real fixture directory is empty (never refreshed), the suite skips.
 */

import * as fs from 'fs';
import * as path from 'path';
import * as yaml from 'js-yaml';
import { pgPool, closePool } from './setup';
import { hasRealFixtures, loadRealFixtures, Transform2Row } from '../fixtures/loader';

const REPO_ROOT = path.resolve(__dirname, '..', '..');
const PIPELINE_PATH = path.join(REPO_ROOT, 'registry-turbo-v4.yaml');

interface PipelineDoc {
  transforms: Record<string, { sql: string; type: string }>;
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

const describeIfReal = hasRealFixtures() ? describe : describe.skip;

describeIfReal('v4 pipeline transforms (real testnet data)', () => {
  let transforms: PipelineDoc['transforms'];
  let rowsByCommand: Record<string, Transform2Row[]>;

  beforeAll(async () => {
    transforms = loadTransforms();
    const realRows = loadRealFixtures();
    rowsByCommand = realRows.reduce<Record<string, Transform2Row[]>>((acc, r) => {
      (acc[r.command] ??= []).push(r);
      return acc;
    }, {});
    await seedTransform2(realRows);
  });

  afterAll(async () => {
    await closePool();
  });

  function expectAllNonNull(rows: any[], columns: string[]) {
    for (const row of rows) {
      for (const col of columns) {
        expect(row[col]).not.toBeNull();
        expect(row[col]).not.toBe(undefined);
      }
    }
  }

  test('deploy events: every row yields all extracted fields', async () => {
    const input = rowsByCommand['deploy'] ?? [];
    if (input.length === 0) return;
    const result = await pgPool().query(transforms.transform_3_deploy_events.sql);
    expect(result.rows.length).toBe(input.length);
    expectAllNonNull(result.rows, ['wasm_name', 'wasm_version', 'deployer', 'contract_id']);
  });

  test('publish events: every row yields all extracted fields', async () => {
    const input = rowsByCommand['publish'] ?? [];
    if (input.length === 0) return;
    const result = await pgPool().query(transforms.transform_3_publish_events.sql);
    expect(result.rows.length).toBe(input.length);
    expectAllNonNull(result.rows, ['author', 'wasm_version', 'wasm_hash', 'wasm_name']);
  });

  test('register events: every row yields all extracted fields', async () => {
    const input = rowsByCommand['register'] ?? [];
    if (input.length === 0) return;
    const result = await pgPool().query(transforms.transform_3_register_events.sql);
    expect(result.rows.length).toBe(input.length);
    // sac is nullable-ish (BOOLEAN) — just assert the row exists and sac is a real bool
    for (const row of result.rows) {
      expect(row.contract_name).not.toBeNull();
      expect(row.contract_id).not.toBeNull();
      expect(typeof row.sac).toBe('boolean');
    }
  });

  test('sub_reg events: all rows from the current root emitter pass the filter', async () => {
    const input = rowsByCommand['sub_reg'] ?? [];
    if (input.length === 0) return;
    const result = await pgPool().query(transforms.transform_3_subregistry_events.sql);
    expect(result.rows.length).toBe(input.length);
    expectAllNonNull(result.rows, ['contract_id', 'channel']);
  });

  test('rename events: every row yields old_name + new_name', async () => {
    const input = rowsByCommand['rename'] ?? [];
    if (input.length === 0) return;
    const result = await pgPool().query(transforms.transform_3_rename.sql);
    expect(result.rows.length).toBe(input.length);
    expectAllNonNull(result.rows, ['old_name', 'new_name']);
  });

  test('update_address events: every row yields contract_name + new_address', async () => {
    const input = rowsByCommand['update_address'] ?? [];
    if (input.length === 0) return;
    const result = await pgPool().query(transforms.transform_3_update_address.sql);
    expect(result.rows.length).toBe(input.length);
    expectAllNonNull(result.rows, ['contract_name', 'new_address']);
  });

  test('update_owner events: every row yields contract_name + new_owner', async () => {
    const input = rowsByCommand['update_owner'] ?? [];
    if (input.length === 0) return;
    const result = await pgPool().query(transforms.transform_3_update_owner.sql);
    expect(result.rows.length).toBe(input.length);
    expectAllNonNull(result.rows, ['contract_name', 'new_owner']);
  });
});
