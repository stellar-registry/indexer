import { pgPool, closePool, applySql } from './setup';

describe('v4 named views', () => {
  beforeAll(async () => {
    const pool = pgPool();
    // Clean slate so repeated runs don't collide on PKs.
    await pool.query(`
      DROP VIEW IF EXISTS v4_published_wasms_named;
      DROP VIEW IF EXISTS v4_registered_contracts_named;
      DROP TABLE IF EXISTS v4_registries;
      DROP TABLE IF EXISTS v4_deployed_contracts;
      DROP TABLE IF EXISTS v4_published_wasms;
      DROP TABLE IF EXISTS v4_registered_contracts;
      DROP TABLE IF EXISTS v4_rename;
      DROP TABLE IF EXISTS v4_update_address;
      DROP TABLE IF EXISTS v4_update_owner;
      DROP TABLE IF EXISTS v4_raw_events_backup;
    `);

    await applySql('sql/v4_sink_tables.sql');
    await applySql('sql/v4_registries.sql');
    await applySql('sql/v4_named_views.sql');
  });

  afterAll(async () => {
    await closePool();
  });

  beforeEach(async () => {
    await pgPool().query(`
      TRUNCATE v4_registries, v4_published_wasms, v4_registered_contracts RESTART IDENTITY;
    `);
  });

  test('resolved_channel maps registered contract_id to friendly channel name', async () => {
    const pool = pgPool();
    const subRegId = 'CSUBREG1000000000000000000000000000000000000000000000';
    await pool.query(
      `INSERT INTO v4_registries (contract_id, channel, id, transaction_hash, ledger_sequence, created_at)
       VALUES ($1, 'soroswap', 'reg-1', 'tx-1', 2038160, NOW())`,
      [subRegId],
    );
    await pool.query(
      `INSERT INTO v4_published_wasms (id, transaction_hash, ledger_sequence, created_at, channel, author, wasm_version, wasm_hash, wasm_name)
       VALUES ('pub-1', 'tx-1', 2038170, NOW(), $1, 'GAUTH', '1.0.0', 'deadbeef', 'wasm_a')`,
      [subRegId],
    );

    const result = await pool.query(
      `SELECT resolved_channel FROM v4_published_wasms_named WHERE id = 'pub-1'`,
    );
    expect(result.rows[0].resolved_channel).toBe('soroswap');
  });

  test('resolved_channel falls back to raw contract_id when no registry entry exists', async () => {
    const pool = pgPool();
    const unknownId = 'CUNKNOWN0000000000000000000000000000000000000000000000';
    await pool.query(
      `INSERT INTO v4_published_wasms (id, transaction_hash, ledger_sequence, created_at, channel, author, wasm_version, wasm_hash, wasm_name)
       VALUES ('pub-2', 'tx-2', 2038171, NOW(), $1, 'GAUTH', '1.0.0', 'cafebabe', 'wasm_b')`,
      [unknownId],
    );

    const result = await pool.query(
      `SELECT resolved_channel FROM v4_published_wasms_named WHERE id = 'pub-2'`,
    );
    expect(result.rows[0].resolved_channel).toBe(unknownId);
  });

  test('v4_registered_contracts_named resolves channel via the same registry', async () => {
    const pool = pgPool();
    const subRegId = 'CSUBREG2000000000000000000000000000000000000000000000';
    await pool.query(
      `INSERT INTO v4_registries (contract_id, channel, id, transaction_hash, ledger_sequence, created_at)
       VALUES ($1, 'aquarius', 'reg-2', 'tx-r', 2038200, NOW())`,
      [subRegId],
    );
    await pool.query(
      `INSERT INTO v4_registered_contracts (id, transaction_hash, ledger_sequence, created_at, channel, contract_name, contract_id, sac, wasm_hash)
       VALUES ('regc-1', 'tx-rc', 2038201, NOW(), $1, 'amm', 'CAMM', true, 'hash')`,
      [subRegId],
    );

    const result = await pool.query(
      `SELECT resolved_channel FROM v4_registered_contracts_named WHERE id = 'regc-1'`,
    );
    expect(result.rows[0].resolved_channel).toBe('aquarius');
  });
});
