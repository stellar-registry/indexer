export const TablesName = {
  // Managed by goldksky, do not drop!
  events: "public.events",
  // Managed by lambda, safe to drop old versions
  deploys: "public.deploys_4"
}

export interface DbEvent {
  id: string,
  type: string,
  topics: string,
  data: string,
  transaction_hash: string,
  in_successful_contract_call: boolean,
  transaction_successful: boolean,
  ledger_sequence: number,
  ledger_closed_at: Date
}

export interface DeployData {
  id: string,
  transaction_hash: string,
  ledger_sequence: number,
  created_at: Date,
  contract_id: string,
  contract_name: string,
  deployer: string,
  version: string,
  wasm_name: string,
}

export interface LedgerSeq {
  ledger_sequence: number,
}
