// Debounced editor saves that haven't fired yet, keyed by tab id. Lets code
// that hands a tab to another window (pop-out) write the latest content first.

const pending = new Map<string, () => Promise<void>>();

export function setPendingAutosave(tabId: string, save: () => Promise<void>): void {
  pending.set(tabId, save);
}

export function clearPendingAutosave(tabId: string): void {
  pending.delete(tabId);
}

/** Run the tab's pending save now, if there is one. */
export async function flushPendingAutosave(tabId: string): Promise<void> {
  const save = pending.get(tabId);
  if (!save) return;
  pending.delete(tabId);
  await save();
}
