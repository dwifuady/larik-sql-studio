import { describe, it, expect, vi, beforeEach } from 'vitest';
import { invokeSafe, ApiErrorException } from './client';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

import { invoke } from '@tauri-apps/api/core';

const mockedInvoke = invoke as unknown as ReturnType<typeof vi.fn>;

describe('invokeSafe', () => {
  beforeEach(() => vi.resetAllMocks());

  it('resolves on success', async () => {
    mockedInvoke.mockResolvedValue('ok');
    await expect(invokeSafe('cmd')).resolves.toBe('ok');
  });

  it('throws ApiErrorException for ApiError object', async () => {
    const err = { code: 'Validation', message: 'bad' };
    mockedInvoke.mockRejectedValue(err);
    await expect(invokeSafe('cmd')).rejects.toBeInstanceOf(ApiErrorException);
    try {
      await invokeSafe('cmd');
    } catch (e) {
      expect((e as ApiErrorException).code).toBe('Validation');
      expect((e as ApiErrorException).message).toBe('bad');
    }
  });

  it('normalizes string error', async () => {
    mockedInvoke.mockRejectedValue('oops');
    await expect(invokeSafe('cmd')).rejects.toMatchObject({ code: 'Unknown', message: 'oops' });
  });

  it('normalizes Error object', async () => {
    mockedInvoke.mockRejectedValue(new Error('boom'));
    await expect(invokeSafe('cmd')).rejects.toMatchObject({ message: 'boom' });
  });

  it('handles object without code/message', async () => {
    mockedInvoke.mockRejectedValue({ foo: 'bar' });
    await expect(invokeSafe('cmd')).rejects.toBeInstanceOf(ApiErrorException);
  });
});
