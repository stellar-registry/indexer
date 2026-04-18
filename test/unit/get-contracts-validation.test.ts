import { validateParams } from '../../lib/get-contracts';

describe('validateParams', () => {
  test('returns defaults when no queryStringParameters', () => {
    const result = validateParams({});
    expect(result).toEqual({ limit: 200, ledger: 0, cursor: '' });
  });

  test('accepts a valid limit', () => {
    const result = validateParams({ queryStringParameters: { limit: '50' } });
    expect(result).toEqual({ limit: 50, ledger: 0, cursor: '' });
  });

  test('rejects limit below the minimum', () => {
    const result = validateParams({ queryStringParameters: { limit: '1' } });
    expect(result).toMatchObject({
      statusCode: 400,
      body: expect.stringContaining('Limit must be an integer between 2 and 200'),
    });
  });

  test('rejects limit above the maximum', () => {
    const result = validateParams({ queryStringParameters: { limit: '500' } });
    expect(result).toMatchObject({ statusCode: 400 });
  });

  test('rejects non-integer limit', () => {
    const result = validateParams({ queryStringParameters: { limit: 'abc' } });
    expect(result).toMatchObject({ statusCode: 400 });
  });

  test('parses a valid cursor and rewrites the tail to lexicographic-max marker', () => {
    const result = validateParams({
      queryStringParameters: { cursor: '2038100-abcd-op-0-event-0' },
    });
    expect(result).toEqual({ limit: 200, ledger: 2038100, cursor: '2038100-abcd-z' });
  });

  test('rejects a cursor with no separator', () => {
    const result = validateParams({ queryStringParameters: { cursor: 'unsplit' } });
    expect(result).toMatchObject({ statusCode: 400 });
  });

  test('rejects a cursor whose ledger component is not a number', () => {
    const result = validateParams({ queryStringParameters: { cursor: 'foo-bar' } });
    expect(result).toMatchObject({
      statusCode: 400,
      body: expect.stringContaining('Invalid cursor'),
    });
  });
});
