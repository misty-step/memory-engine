import { describe, expect, test } from 'bun:test';

describe('scaffold smoke test', () => {
  test('test runner is wired up', () => {
    expect(1 + 1).toBe(2);
  });
});
