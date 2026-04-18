import { parseDeploy } from '../../lib/periodic-lambda';
import { DbEvent } from '../../lib/db.types';

function makeEvent(dataMap: Array<{ key: { symbol: string }; val: any }>): DbEvent {
  return {
    id: 'event-id-1',
    type: 'contract',
    topics: JSON.stringify([{ symbol: 'deploy' }]),
    data: JSON.stringify({ map: dataMap }),
    transaction_hash: 'tx-hash-1',
    in_successful_contract_call: true,
    transaction_successful: true,
    ledger_sequence: '2038101',
    ledger_closed_at: new Date('2026-04-01T00:00:00Z'),
  };
}

describe('parseDeploy', () => {
  test('extracts all fields when keys are in canonical order', () => {
    const event = makeEvent([
      { key: { symbol: 'contract_id' }, val: { address: 'CDEPLOYED' } },
      { key: { symbol: 'contract_name' }, val: { string: 'my_contract' } },
      { key: { symbol: 'deployer' }, val: { address: 'GDEPLOYER' } },
      { key: { symbol: 'version' }, val: { string: '1.0.0' } },
      { key: { symbol: 'wasm_name' }, val: { string: 'my_wasm' } },
    ]);

    const result = parseDeploy(event);

    expect(result.contract_id).toBe('CDEPLOYED');
    expect(result.contract_name).toBe('my_contract');
    expect(result.deployer).toBe('GDEPLOYER');
    expect(result.version).toBe('1.0.0');
    expect(result.wasm_name).toBe('my_wasm');
    expect(result.id).toBe('event-id-1');
    expect(result.transaction_hash).toBe('tx-hash-1');
    expect(result.ledger_sequence).toBe('2038101');
  });

  test('extracts fields regardless of map-key order', () => {
    const event = makeEvent([
      { key: { symbol: 'wasm_name' }, val: { string: 'reordered' } },
      { key: { symbol: 'version' }, val: { string: '9.9.9' } },
      { key: { symbol: 'deployer' }, val: { address: 'GOTHER' } },
      { key: { symbol: 'contract_id' }, val: { address: 'CREORDER' } },
      { key: { symbol: 'contract_name' }, val: { string: 'reordered_contract' } },
    ]);

    const result = parseDeploy(event);

    expect(result).toMatchObject({
      contract_id: 'CREORDER',
      contract_name: 'reordered_contract',
      deployer: 'GOTHER',
      version: '9.9.9',
      wasm_name: 'reordered',
    });
  });

  test('unknown symbols are ignored without throwing', () => {
    const spy = jest.spyOn(console, 'log').mockImplementation(() => {});
    const event = makeEvent([
      { key: { symbol: 'wasm_name' }, val: { string: 'w' } },
      { key: { symbol: 'surprise' }, val: { string: 'x' } },
    ]);

    const result = parseDeploy(event);

    expect(result.wasm_name).toBe('w');
    expect(spy).toHaveBeenCalledWith('Unexpected symbol', 'surprise');
    spy.mockRestore();
  });

  test('missing keys default to empty strings', () => {
    const event = makeEvent([
      { key: { symbol: 'contract_id' }, val: { address: 'CONLY' } },
    ]);

    const result = parseDeploy(event);

    expect(result.contract_id).toBe('CONLY');
    expect(result.contract_name).toBe('');
    expect(result.deployer).toBe('');
    expect(result.version).toBe('');
    expect(result.wasm_name).toBe('');
  });
});
