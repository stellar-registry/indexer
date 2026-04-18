import * as fs from 'fs';
import * as path from 'path';

export interface Transform2Row {
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

const SYNTH_DIR = path.join(__dirname, 'soroban-events');
const REAL_DIR = path.join(__dirname, 'soroban-events-real');

function loadDir(dir: string): Transform2Row[] {
  if (!fs.existsSync(dir)) return [];
  const files = fs
    .readdirSync(dir)
    .filter((f) => f.endsWith('.json'))
    .sort();
  return files.flatMap(
    (f) => JSON.parse(fs.readFileSync(path.join(dir, f), 'utf8')) as Transform2Row[],
  );
}

export function loadFixture(symbol: string): Transform2Row[] {
  return JSON.parse(
    fs.readFileSync(path.join(SYNTH_DIR, `${symbol}.json`), 'utf8'),
  ) as Transform2Row[];
}

export function loadAllFixtures(): Transform2Row[] {
  return loadDir(SYNTH_DIR);
}

export function loadRealFixtures(): Transform2Row[] {
  return loadDir(REAL_DIR);
}

export function hasRealFixtures(): boolean {
  return fs.existsSync(REAL_DIR) && fs.readdirSync(REAL_DIR).length > 0;
}
