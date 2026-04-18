/**
 * Fetch real Soroban testnet events for the root registry contract and write
 * them to test/fixtures/soroban-events-real/<symbol>.json in the shape expected
 * by the pipeline's transform_2 output.
 *
 * Invoked via `npm run fixtures:refresh`. Not part of CI — requires network.
 * Re-run when the event schema changes or a new event symbol is introduced.
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

async function main(): Promise<void> {
  const server = new SorobanRpc.Server(RPC_URL);
  const latest = await server.getLatestLedger();
  const startLedger = Math.max(latest.sequence - LOOKBACK_LEDGERS, 1);

  console.log(
    `fetching events for ${CURRENT_ROOT} from ledger ${startLedger} → ${latest.sequence}`,
  );

  const events = await fetchFor(CURRENT_ROOT, startLedger);
  console.log(`decoded ${events.length} raw events`);
  if (events.length === 0) {
    console.log('no events for the current root in the retention window');
    console.log('fixtures will be empty; real-data test suite will skip');
  }

  const bySymbol: Record<string, unknown[]> = {};
  const toScVal = (raw: unknown): xdr.ScVal =>
    raw instanceof xdr.ScVal ? raw : xdr.ScVal.fromXDR(raw as string, 'base64');

  for (const ev of events) {
    const topicsJson = (ev.topic as unknown[]).map((t) => scvalToGoldsky(toScVal(t)));
    const dataJson = scvalToGoldsky(toScVal(ev.value));
    const head = topicsJson[0] as { symbol?: string } | undefined;
    const symbol = head?.symbol;
    if (!symbol) continue;
    const rawContractId = (ev as { contractId?: unknown }).contractId;
    const emitter =
      typeof rawContractId === 'string' ? rawContractId : String(rawContractId ?? '');
    const row = {
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

  fs.mkdirSync(FIXTURE_DIR, { recursive: true });
  // Clear stale per-symbol files so a shrinking event set doesn't leave
  // leftovers from a previous refresh.
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

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
