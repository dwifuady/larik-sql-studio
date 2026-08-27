import { invoke } from '@tauri-apps/api/core';

export interface ApiError {
  code: string;
  message: string;
}

export class ApiErrorException extends Error {
  code: string;
  constructor(e: ApiError) {
    super(e.message);
    this.code = e.code;
    this.name = 'ApiErrorException';
  }
}

export async function invokeSafe<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e: unknown) {
    // Tauri rejects with either the serialized ApiError object or a plain string.
    if (e && typeof e === 'object' && 'code' in e && 'message' in e) {
      throw new ApiErrorException(e as ApiError);
    }
    const message =
      typeof e === 'string' ? e : (e as Error)?.message ?? String(e);
    throw new ApiErrorException({ code: 'Unknown', message });
  }
}
