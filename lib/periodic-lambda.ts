import { Handler } from "aws-cdk-lib/aws-lambda";
import { EventBridgeEvent } from 'aws-lambda';
import { SecretsManagerClient, GetSecretValueCommand } from "@aws-sdk/client-secrets-manager";
import { Pool, QueryResult } from 'pg';
import { DbEvent, DeployData, LedgerSeq, TablesName } from "./db.types"

const client = new SecretsManagerClient();

let last_deploy_event_seq: number;
let pool: Pool;

export const handler: Handler = async (event: EventBridgeEvent<string, any>): Promise<void> => {
  console.log("Triggered by Rule ID:", event.id);

  const pool = await getPool();
  const last_ledger = await getLastLedgerSeq();

  const query = {
    text: `SELECT id, type, topics, data, transaction_hash, in_successful_contract_call, 
    transaction_successful, ledger_sequence, ledger_closed_at 
    FROM ${TablesName.events} 
    WHERE ledger_sequence > $1`,
    values: [last_ledger],
  };

  const result: QueryResult<DbEvent> = await pool.query(query);
  const events: DbEvent[] = result.rows;

  console.log("Loaded events: ", JSON.stringify(events))

  let newDeploys = [];

  for (const event of events) {
    const topics = JSON.parse(event.topics)
    console.log("event topic: ", topics)
    if (topics[0].symbol == "deploy") {
      newDeploys.push(parseDeploy(event))
    }
  }

  await insertDeploys(newDeploys, pool)

  console.log("Periodic Job Completed");
};

async function getPool(): Promise<Pool> {
  if (pool) return pool; // Return existing pool if already initialized

  const response = await client.send(
    new GetSecretValueCommand({ SecretId: "goldsky-pg-url" })
  );

  const dbUrl = response.SecretString

  pool = new Pool({
    connectionString: dbUrl,
    ssl: { rejectUnauthorized: false }
  });

  return pool;
}

async function getLastLedgerSeq(): Promise<number> {
  if (last_deploy_event_seq) {
    return last_deploy_event_seq;
  }
  const last_known_ledger_query = {
    text: `select ledger_sequence from ${TablesName.deploys} order by ledger_sequence desc limit 1`
  }

  const res: QueryResult<LedgerSeq> = await pool.query(last_known_ledger_query)

  let last_ledger = 0
  if (res.rows.length != 0) {
    last_ledger = res.rows[0].ledger_sequence
  }

  console.log("Loaded last deploy event ledger: ", last_ledger)

  last_deploy_event_seq = last_ledger;

  return last_deploy_event_seq;
}

function parseDeploy(event: DbEvent): DeployData {
  const data = JSON.parse(event.data)
  const elements = data.map

  const deployData: DeployData = {
    created_at: new Date(),
    contract_id: "",
    contract_name: "",
    deployer: "",
    version: "",
    wasm_name: "",
    ...event
  }

  for (const elem of elements) {
    switch (elem.key.symbol) {
      case "contract_id": {
        deployData.contract_id = elem.val.address
        break
      }
      case "contract_name": {
        deployData.contract_name = elem.val.string
        break
      }
      case "deployer": {
        deployData.deployer = elem.val.address
        break
      }
      case "version": {
        deployData.version = elem.val.string
        break
      }
      case "wasm_name": {
        deployData.wasm_name = elem.val.string
        break
      }
      default: console.log("Unexpected symbol", elem.key.symbol)
    }
  }
  return deployData
}

async function insertDeploys(newDeploys: Array<DeployData>, pool: Pool): Promise<void> {
  if (newDeploys.length > 0) {
    console.log("deploys to insert:", JSON.stringify(newDeploys))

    const ids = newDeploys.map(d => d.id);
    const tx_hashes = newDeploys.map(d => d.transaction_hash);
    const l_seqs = newDeploys.map(d => d.ledger_sequence);
    const created_ats = newDeploys.map(d => d.created_at);
    const contract_ids = newDeploys.map(d => d.contract_id);
    const contract_names = newDeploys.map(d => d.contract_name);
    const deployers = newDeploys.map(d => d.deployer);
    const versions = newDeploys.map(d => d.version);
    const wasm_names = newDeploys.map(d => d.wasm_name);
    const nows = newDeploys.map(_ => "NOW()")

    const query = `
      INSERT INTO ${TablesName.deploys} (
        id, transaction_hash, ledger_sequence, created_at, contract_id, 
        contract_name, deployer, version, wasm_name, processed_at)
      SELECT * FROM UNNEST(
        $1::text[], $2::text[], $3::bigint[], $4::timestamp[], $5::text[], 
        $6::text[], $7::text[], $8::text[], $9::text[], $10::timestamp[])
      RETURNING id;
    `;

    const values = [ids, tx_hashes, l_seqs, created_ats, contract_ids, contract_names, deployers, versions, wasm_names, nows];
    const res = await pool.query(query, values);
    console.log(`inserted ${res.rowCount} rows`);
  }
}
