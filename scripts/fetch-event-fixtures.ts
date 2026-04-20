/**
 * Fetch real Soroban testnet events for the root registry contract and write
 * them to test/fixtures/soroban-events-real/<symbol>.json in the shape expected
 * by the pipeline's transform_2 output.
 *
 * Usage:
 *   npm run fixtures:refresh          — wipe & replace all real fixtures
 *   npm run fixtures:pull             — fetch only events newer than what we have
 *
 * Not part of CI — requires network.
 *
 * The script only fetches events actually emitted by the current root. Soroban
 * testnet RPC only retains ~10-20k ledgers of event history, so we scan a
 * recent window. If the root has no events in that window (e.g. just after a
 * rotation with no activity), the fixtures will be empty — the real-data test
 * suite then skips. Synthetic fixtures always provide coverage.
 */

import * as fs from 'fs';
import * as path from 'path';
import { rpc as SorobanRpc, xdr, Address } from '@stellar/stellar-sdk';

const RPC_URL = 'https://soroban-testnet.stellar.org';
const CURRENT_ROOT = 'CBNBQND6EMYTTRTCUWUJ3VIKF7RUUISK5T4GAKTXRVIQRHGP4XQY4ID7';
// How many ledgers back to scan. Soroban testnet RPC retention is ~10k
// ledgers (measured empirically ~17 hours at 6s/ledger). Values outside that
// window are silently rejected — the RPC returns an empty event list.
const LOOKBACK_LEDGERS = 9_000;
const FIXTURE_DIR = path.join(__dirname, '..', 'test', 'fixtures', 'soroban-events-real');

function scvalToGoldsky(sv: xdr.ScVal): unknown {
  const tag = sv.switch().name;
  switch (tag) {
    case 'scvSymbol':
      return { symbol: sv.sym().toString() };
    case 'scvString':
      return { string: sv.str().toString() };
    case 'scvBool':
      return { bool: sv.b() };
    case 'scvBytes':
      return { bytes: sv.bytes().toString('hex') };
    case 'scvAddress':
      return { address: Address.fromScAddress(sv.address()).toString() };
    case 'scvMap':
      return {
        map: (sv.map() ?? []).map((e) => ({
          key: scvalToGoldsky(e.key()),
          val: scvalToGoldsky(e.val()),
        })),
      };
    case 'scvVec':
      return { vec: (sv.vec() ?? []).map(scvalToGoldsky) };
    case 'scvU32':
      return { u32: sv.u32() };
    case 'scvI32':
      return { i32: sv.i32() };
    case 'scvU64':
      return { u64: sv.u64().toString() };
    case 'scvI64':
      return { i64: sv.i64().toString() };
    case 'scvVoid':
      return { void: null };
    default:
      return { unknown: tag };
  }
}

async function fetchFor(contractId: string, startLedger: number) {
  const server = new SorobanRpc.Server(RPC_URL);
  const resp = await server.getEvents({
    startLedger,
    filters: [{ type: 'contract', contractIds: [contractId] }],
    limit: 1000,
  });
  return resp.events;
}

interface FixtureRow {
  id: string;
  transaction_hash: string;
  ledger_sequence: number;
  created_at: string;
  command: string;
  channel: string;
  emitter_contract_id: string;
  data: string;
  topics: string;
}

const toScVal = (raw: unknown): xdr.ScVal =>
  raw instanceof xdr.ScVal ? raw : xdr.ScVal.fromXDR(raw as string, 'base64');

/** Convert raw RPC events into fixture rows grouped by command symbol. */
function eventsToRows(events: Awaited<ReturnType<typeof fetchFor>>): Record<string, FixtureRow[]> {
  const bySymbol: Record<string, FixtureRow[]> = {};
  for (const ev of events) {
    const topicsJson = (ev.topic as unknown[]).map((t) => scvalToGoldsky(toScVal(t)));
    const dataJson = scvalToGoldsky(toScVal(ev.value));
    const head = topicsJson[0] as { symbol?: string } | undefined;
    const symbol = head?.symbol;
    if (!symbol) continue;
    const rawContractId = (ev as { contractId?: unknown }).contractId;
    const emitter =
      typeof rawContractId === 'string' ? rawContractId : String(rawContractId ?? '');
    const row: FixtureRow = {
      id: ev.id,
      transaction_hash: ev.txHash,
      ledger_sequence: Number(ev.ledger),
      created_at: ev.ledgerClosedAt,
      command: symbol,
      channel: emitter,
      emitter_contract_id: emitter,
      data: JSON.stringify(dataJson),
      topics: JSON.stringify(topicsJson),
    };
    (bySymbol[symbol] ??= []).push(row);
  }
  return bySymbol;
}

/** Read existing fixture files and return the highest ledger_sequence seen. */
function loadExistingFixtures(): { bySymbol: Record<string, FixtureRow[]>; maxLedger: number } {
  const bySymbol: Record<string, FixtureRow[]> = {};
  let maxLedger = 0;
  if (!fs.existsSync(FIXTURE_DIR)) return { bySymbol, maxLedger };
  for (const file of fs.readdirSync(FIXTURE_DIR).filter((f) => f.endsWith('.json'))) {
    const symbol = path.basename(file, '.json');
    const rows: FixtureRow[] = JSON.parse(fs.readFileSync(path.join(FIXTURE_DIR, file), 'utf8'));
    bySymbol[symbol] = rows;
    for (const r of rows) {
      if (r.ledger_sequence > maxLedger) maxLedger = r.ledger_sequence;
    }
  }
  return { bySymbol, maxLedger };
}

/** Extract sub-registry contract IDs from sub_reg fixture rows. */
function extractSubRegistryIds(rows: FixtureRow[]): string[] {
  const ids: string[] = [];
  for (const row of rows) {
    const data = JSON.parse(row.data) as {
      map?: { key: { symbol?: string }; val: { address?: string } }[];
    };
    for (const entry of data.map ?? []) {
      if (entry.key.symbol === 'contract_id' && entry.val.address) {
        ids.push(entry.val.address);
      }
    }
  }
  return ids;
}

/** Fetch events for the root contract and all known sub-registries. */
async function fetchAllContracts(
  startLedger: number,
  existingSubRegRows?: FixtureRow[],
): Promise<Awaited<ReturnType<typeof fetchFor>>> {
  // Fetch root events first.
  const rootEvents = await fetchFor(CURRENT_ROOT, startLedger);
  console.log(`  root: ${rootEvents.length} events`);

  // Discover sub-registry contract IDs from both freshly fetched and
  // previously existing sub_reg rows.
  const freshRows = eventsToRows(rootEvents);
  const allSubRegRows = [
    ...(freshRows['sub_reg'] ?? []),
    ...(existingSubRegRows ?? []),
  ];
  const subIds = [...new Set(extractSubRegistryIds(allSubRegRows))]
    .filter((id) => id !== CURRENT_ROOT);

  // Fetch events from each sub-registry in parallel.
  const subResults = await Promise.all(
    subIds.map(async (id) => {
      const events = await fetchFor(id, startLedger);
      console.log(`  sub-registry ${id}: ${events.length} events`);
      return events;
    }),
  );

  return [...rootEvents, ...subResults.flat()];
}

/** Write grouped fixture rows to disk. */
function writeFixtures(bySymbol: Record<string, FixtureRow[]>): void {
  fs.mkdirSync(FIXTURE_DIR, { recursive: true });
  for (const existing of fs.readdirSync(FIXTURE_DIR)) {
    if (existing.endsWith('.json')) {
      fs.unlinkSync(path.join(FIXTURE_DIR, existing));
    }
  }
  for (const [symbol, rows] of Object.entries(bySymbol)) {
    const file = path.join(FIXTURE_DIR, `${symbol}.json`);
    fs.writeFileSync(file, JSON.stringify(rows, null, 2) + '\n');
    console.log(`wrote ${rows.length} ${symbol} events → ${file}`);
  }
}

/** Wipe and replace: fetch everything in the lookback window. */
async function refresh(): Promise<void> {
  const server = new SorobanRpc.Server(RPC_URL);
  const latest = await server.getLatestLedger();
  const startLedger = Math.max(latest.sequence - LOOKBACK_LEDGERS, 1);

  console.log(
    `fetching events from ledger ${startLedger} → ${latest.sequence}`,
  );

  const events = await fetchAllContracts(startLedger);
  console.log(`decoded ${events.length} total events`);
  if (events.length === 0) {
    console.log('no events in the retention window');
    console.log('fixtures will be empty; real-data test suite will skip');
  }

  writeFixtures(eventsToRows(events));
}

/** Incremental pull: keep existing fixtures and append only newer events. */
async function pull(): Promise<void> {
  const { bySymbol: existing, maxLedger } = loadExistingFixtures();
  const existingIds = new Set<string>();
  for (const rows of Object.values(existing)) {
    for (const r of rows) existingIds.add(r.id);
  }

  const server = new SorobanRpc.Server(RPC_URL);
  const latest = await server.getLatestLedger();
  // Start one ledger after the last one we already have, but never older
  // than the RPC retention window (the RPC silently returns nothing if the
  // startLedger is outside its ~10k-ledger window).
  const retentionFloor = Math.max(latest.sequence - LOOKBACK_LEDGERS, 1);
  const startLedger = maxLedger > 0
    ? Math.max(maxLedger + 1, retentionFloor)
    : retentionFloor;

  console.log(
    `pulling new events from ledger ${startLedger} → ${latest.sequence}` +
      (maxLedger > 0 ? ` (existing max ledger: ${maxLedger})` : ' (no existing fixtures, full fetch)'),
  );

  const events = await fetchAllContracts(startLedger, existing['sub_reg']);
  const newRows = eventsToRows(events);

  // Deduplicate by event id and merge into existing fixtures.
  let added = 0;
  for (const [symbol, rows] of Object.entries(newRows)) {
    if (!existing[symbol]) existing[symbol] = [];
    for (const row of rows) {
      if (!existingIds.has(row.id)) {
        existing[symbol].push(row);
        existingIds.add(row.id);
        added++;
      }
    }
  }

  console.log(`fetched ${events.length} events, ${added} new`);
  writeFixtures(existing);
}

async function main(): Promise<void> {
  const mode = process.argv[2] ?? 'refresh';
  if (mode === 'pull') {
    await pull();
  } else {
    await refresh();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
