import { Handler } from "aws-cdk-lib/aws-lambda";
import { SecretsManagerClient, GetSecretValueCommand } from "@aws-sdk/client-secrets-manager";
import { Pool, QueryResult } from 'pg';
import { DbEvent, DeployData, LedgerSeq, PublishData, TablesName, query_limit } from "./db.types"

const client = new SecretsManagerClient();

let pool: Pool;

export const handler: Handler = async (event: any) => {
  const pool = await getPool();

  let limit = 200;
  let cursor = '';
  let ledger = 0;
  let next = null;

  if (event.queryStringParameters) {
    if (event.queryStringParameters.limit) {
      limit = Number(event.queryStringParameters.limit)
      if (!Number.isInteger(limit) || limit < 2 || limit > 200) {
        return {
          statusCode: 400,
          body: JSON.stringify({ error: "Limit must be an integer between 2 and 200" })
        }
      }
    }
    if (event.queryStringParameters.cursor) {
      cursor = event.queryStringParameters.cursor;
      const split = cursor.split('-')
      if (split.length == 0) {
        return {
          statusCode: 400,
          body: JSON.stringify({ error: "Invalid cursor" })
        }
      }
      ledger = Number(split[0])
      if (!Number.isInteger(ledger) || ledger < 0) {
        return {
          statusCode: 400,
          body: JSON.stringify({ error: "Invalid cursor" })
        }
      }
      // Format is <ledger>-<tx hash>-op-<op number>-event-<event number>
      // See explanation below
      cursor = `${split[0]}-${split[1]}-z`
    }
  }

  const query = {
    // Groups by wasm_name (priority to the latest publish by ledger_sequence)
    // Edgecase: if there are multiple publishes in the same ledger, rely on semver 

    // Finally, all records are sorted first by ledger_sequence (including passed ledger), 
    // and then by id (excluding passed id). Because IDs are strings, we transform passed id 
    // With adding an extra 'z' symbol to ensure string is lexicographically greater 
    // to go to the next transaction in the same ledger (if any)
    text: `select * from 
      (select *, ROW_NUMBER() over
        (PARTITION by wasm_name ORDER BY ledger_sequence, version DESC) as RN 
        from ${TablesName.get_publishes}
      ) where rn=1 and (ledger_sequence, id) >= (${ledger}, '${cursor}') 
      order by ledger_sequence, id asc
      limit ${limit};`,
  };

  const query_result: QueryResult<PublishData> = await pool.query(query);
  const events: PublishData[] = query_result.rows;

  if (events.length == limit) {
    next = events[events.length - 1].id
  }

  const result = events.map(e => {
    return {
      author: e.author,
      version: e.version,
      wasm_name: e.wasm_name,
      wasm_hash: e.wasm_hash
    }
  })

  return {
    statusCode: 200,
    body: JSON.stringify({
      result: result,
      next: next,
    })
  };
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


