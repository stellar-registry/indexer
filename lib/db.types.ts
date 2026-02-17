export const TablesName = {
  // Managed by goldksky, do not drop!
  events: "public.events",
  // Table used by http lambda to fetch data
  get_publishes: "public.publishes_5",
  // Managed by Periodic lambda, safe to drop old versions
  deploys: "public.deploys_5",
  publishes: "public.publishes_5"
}

// Very small limit to test batch loading; TODO: set this value to 5000 for prod
export const query_limit = 5;

export interface DbEvent {
  id: string,
  type: string,
  topics: string,
  data: string,
  transaction_hash: string,
  in_successful_contract_call: boolean,
  transaction_successful: boolean,
  ledger_sequence: string, // bigint in pgsql is a string in ts
  ledger_closed_at: Date
}

export interface DeployData {
  // Common fields
  id: string,
  transaction_hash: string,
  ledger_sequence: string,
  // Deploy event fields
  contract_id: string,
  contract_name: string,
  deployer: string,
  version: string,
  wasm_name: string,
}

export interface PublishData {
  // Common fields
  id: string,
  transaction_hash: string,
  ledger_sequence: string,
  // Publish event fields
  author: string,
  version: string,
  wasm_name: string,
  wasm_hash: string
}

export interface LedgerSeq {
  ledger_sequence: number,
}
