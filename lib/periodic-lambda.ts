import { Handler } from "aws-cdk-lib/aws-lambda";
import { EventBridgeEvent } from 'aws-lambda';
import { SecretsManagerClient, GetSecretValueCommand } from "@aws-sdk/client-secrets-manager";
import { Pool, QueryResult } from 'pg';
import { DbEvent, DeployData, LedgerSeq, PublishData, TablesName, query_limit } from "./db.types"

const client = new SecretsManagerClient();

let fetch_deploys_from_seq: number;
let fetch_publish_from_seq: number;
let pool: Pool;

export const handler: Handler = async (event: EventBridgeEvent<string, any>): Promise<void> => {
  console.log("Triggered by Rule ID:", event.id);

  const pool = await getPool();

  if (!fetch_deploys_from_seq) {
    fetch_deploys_from_seq = await getLastLedgerSeq(TablesName.deploys);
  }
  if (!fetch_publish_from_seq) {
    fetch_publish_from_seq = await getLastLedgerSeq(TablesName.publishes);
  }

  const fetch_from_inclusive = Math.min(fetch_deploys_from_seq, fetch_publish_from_seq);

  console.log(`Checking events from ${fetch_from_inclusive} last known deploy: ${fetch_deploys_from_seq} publish: ${fetch_publish_from_seq}`)

  const query = {
    text: `SELECT id, type, topics, data, transaction_hash, in_successful_contract_call, 
    transaction_successful, ledger_sequence, ledger_closed_at 
    FROM ${TablesName.events} 
    WHERE ledger_sequence >= $1
    ORDER BY ledger_sequence ASC
    LIMIT ${query_limit}`,
    values: [fetch_from_inclusive],
  };

  const result: QueryResult<DbEvent> = await pool.query(query);
  const events: DbEvent[] = result.rows;

  if (events.length == 0) {
    console.log("No new events")
    return;
  }

  console.log("Loaded events: ", JSON.stringify(events))

  let newDeploys = [];
  let newPublishes = [];
  let last_batch_event_seq = 0;

  for (const event of events) {
    const topics = JSON.parse(event.topics)
    console.log(`processing event id ${event.id} with topics ${JSON.stringify(topics)}`)
    if (topics.length > 1) {
      console.error("Unexpected length of topics array")
    }

    const event_seq = Number(event.ledger_sequence)
    last_batch_event_seq = Math.max(last_batch_event_seq, event_seq);

    // Inclusive because of edge case where batch-1 and batch-2 ends and start on the same seq number 
    if (topics[0].symbol == "deploy" && event_seq >= fetch_deploys_from_seq) {
      newDeploys.push(parseDeploy(event))
    }
    if (topics[0].symbol == "publish" && event_seq >= fetch_publish_from_seq) {
      newPublishes.push(parsePublish(event))
    }
  }

  await insertDeploys(newDeploys, pool)
  await insertPublishes(newPublishes, pool)

  fetch_deploys_from_seq = Math.max(fetch_deploys_from_seq, last_batch_event_seq)
  fetch_publish_from_seq = Math.max(fetch_publish_from_seq, last_batch_event_seq)

  if (events.length < query_limit) {
    console.log("Reached end of known events")
    // Can safely set both numbers to the latest sequence + 1 (to not include last known ledger anymore)
    fetch_deploys_from_seq++;
    fetch_publish_from_seq++;
  }

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

async function getLastLedgerSeq(tableName: string): Promise<number> {
  const last_known_ledger_query = {
    text: `select ledger_sequence from ${tableName} order by ledger_sequence desc limit 1`
  }

  const res: QueryResult<LedgerSeq> = await pool.query(last_known_ledger_query)

  let last_ledger = 0
  if (res.rows.length != 0) {
    last_ledger = res.rows[0].ledger_sequence
  }

  console.log(`Loaded last ${tableName} ledger: ${last_ledger}`)

  return last_ledger;
}

export function parseDeploy(event: DbEvent): DeployData {
  const data = JSON.parse(event.data)
  const elements = data.map

  const deployData: DeployData = {
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
  if (newDeploys.length == 0) {
    console.log("No new deploys");
    return
  }
  console.log("deploys to insert:", JSON.stringify(newDeploys))

  const ids = newDeploys.map(d => d.id);
  const tx_hashes = newDeploys.map(d => d.transaction_hash);
  const l_seqs = newDeploys.map(d => Number(d.ledger_sequence));
  const nows = newDeploys.map(_ => "NOW()")
  const contract_ids = newDeploys.map(d => d.contract_id);
  const contract_names = newDeploys.map(d => d.contract_name);
  const deployers = newDeploys.map(d => d.deployer);
  const versions = newDeploys.map(d => d.version);
  const wasm_names = newDeploys.map(d => d.wasm_name);

  // When other table is dropped and is being re-processed, we might process 
  // the same event for deploys again, so do nothing on id conflict
  const query = `
      INSERT INTO ${TablesName.deploys} (
        id, transaction_hash, ledger_sequence, created_at, contract_id, 
        contract_name, deployer, version, wasm_name)
      SELECT * FROM UNNEST(
        $1::text[], $2::text[], $3::bigint[], $4::timestamp[], $5::text[], 
        $6::text[], $7::text[], $8::text[], $9::text[])
      ON CONFLICT (id) DO NOTHING 
      RETURNING id;
    `;

  const values = [ids, tx_hashes, l_seqs, nows, contract_ids, contract_names, deployers, versions, wasm_names];
  const res = await pool.query(query, values);

  console.log(`inserted ${res.rowCount} rows in ${TablesName.deploys}`);
}

export function parsePublish(event: DbEvent): PublishData {
  const data = JSON.parse(event.data)
  const elements = data.map

  const publishData: PublishData = {
    author: "",
    version: "",
    wasm_name: "",
    wasm_hash: "",
    ...event
  }

  for (const elem of elements) {
    switch (elem.key.symbol) {
      case "author": {
        publishData.author = elem.val.address
        break
      }
      case "version": {
        publishData.version = elem.val.string
        break
      }
      case "wasm_name": {
        publishData.wasm_name = elem.val.string
        break
      }
      case "wasm_hash": {
        publishData.wasm_hash = elem.val.bytes
        break
      }
      default: console.log("Unexpected symbol", elem.key.symbol)
    }
  }
  return publishData
}

async function insertPublishes(newPublishes: Array<PublishData>, pool: Pool): Promise<void> {
  if (newPublishes.length == 0) {
    console.log("No new publishes")
    return
  }
  console.log("publishes to insert:", JSON.stringify(newPublishes))

  const ids = newPublishes.map(d => d.id);
  const tx_hashes = newPublishes.map(d => d.transaction_hash);
  const l_seqs = newPublishes.map(d => Number(d.ledger_sequence));
  const nows = newPublishes.map(_ => "NOW()")
  const authors = newPublishes.map(d => d.author);
  const versions = newPublishes.map(d => d.version);
  const wasm_names = newPublishes.map(d => d.wasm_name);
  const wasm_hashes = newPublishes.map(d => d.wasm_hash);

  const query = `
      INSERT INTO ${TablesName.publishes} (
        id, transaction_hash, ledger_sequence, created_at, 
        author, version, wasm_name, wasm_hash)
      SELECT * FROM UNNEST(
        $1::text[], $2::text[], $3::bigint[], $4::timestamp[], 
        $5::text[], $6::text[], $7::text[], $8::text[])
      ON CONFLICT (id) DO NOTHING
      RETURNING id;
    `;

  const values = [ids, tx_hashes, l_seqs, nows, authors, versions, wasm_names, wasm_hashes];
  const res = await pool.query(query, values);

  console.log(`inserted ${res.rowCount} rows in ${TablesName.publishes}`);
}
