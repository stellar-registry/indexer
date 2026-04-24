import { parsePublish } from '../../lib/periodic-lambda';
import { DbEvent } from '../../lib/db.types';

function makeEvent(dataMap: Array<{ key: { symbol: string }; val: any }>): DbEvent {
  return {
    id: 'publish-event-1',
    type: 'contract',
    topics: JSON.stringify([{ symbol: 'publish' }]),
    data: JSON.stringify({ map: dataMap }),
    transaction_hash: 'pub-tx-1',
    in_successful_contract_call: true,
    transaction_successful: true,
    ledger_sequence: '2038110',
    ledger_closed_at: new Date('2026-04-01T00:01:00Z'),
  };
}

describe('parsePublish', () => {
  test('extracts all fields in canonical order', () => {
    const event = makeEvent([
      { key: { symbol: 'author' }, val: { address: 'GAUTHOR' } },
      { key: { symbol: 'version' }, val: { string: '1.2.3' } },
      { key: { symbol: 'wasm_hash' }, val: { bytes: 'deadbeef' } },
      { key: { symbol: 'wasm_name' }, val: { string: 'my_wasm' } },
    ]);

    const result = parsePublish(event);

    expect(result).toMatchObject({
      author: 'GAUTHOR',
      version: '1.2.3',
      wasm_hash: 'deadbeef',
      wasm_name: 'my_wasm',
      id: 'publish-event-1',
      transaction_hash: 'pub-tx-1',
    });
  });

  test('extracts fields regardless of map-key order', () => {
    const event = makeEvent([
      { key: { symbol: 'wasm_name' }, val: { string: 'reordered' } },
      { key: { symbol: 'author' }, val: { address: 'GXYZ' } },
      { key: { symbol: 'wasm_hash' }, val: { bytes: 'cafebabe' } },
      { key: { symbol: 'version' }, val: { string: '0.0.1' } },
    ]);

    const result = parsePublish(event);

    expect(result.author).toBe('GXYZ');
    expect(result.version).toBe('0.0.1');
    expect(result.wasm_hash).toBe('cafebabe');
    expect(result.wasm_name).toBe('reordered');
  });

  test('unknown symbols are ignored without throwing', () => {
    const spy = jest.spyOn(console, 'log').mockImplementation(() => {});
    const event = makeEvent([
      { key: { symbol: 'author' }, val: { address: 'GA' } },
      { key: { symbol: 'mystery' }, val: { string: 'huh' } },
    ]);

    parsePublish(event);

    expect(spy).toHaveBeenCalledWith('Unexpected symbol', 'mystery');
    spy.mockRestore();
  });

  test('missing keys default to empty strings', () => {
    const event = makeEvent([{ key: { symbol: 'author' }, val: { address: 'GONLY' } }]);

    const result = parsePublish(event);

    expect(result.author).toBe('GONLY');
    expect(result.version).toBe('');
    expect(result.wasm_name).toBe('');
    expect(result.wasm_hash).toBe('');
  });
});
