/// <reference types="vite/client" />

// requestIdleCallback types (not in standard lib)
interface IdleDeadline {
  readonly didTimeout: boolean;
  timeRemaining(): DOMHighResTimeStamp;
}

interface IdleRequestOptions {
  timeout?: number;
}

interface Window {
  requestIdleCallback(callback: (deadline: IdleDeadline) => void, options?: IdleRequestOptions): number;
  cancelIdleCallback(handle: number): void;
}
