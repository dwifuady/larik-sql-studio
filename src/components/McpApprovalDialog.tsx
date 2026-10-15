import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { ShieldAlert, X } from 'lucide-react';
import { getPendingMcpApprovals, respondMcpApproval } from '../api';
import type { McpApprovalRequest } from '../types';

/**
 * Human gate for MCP write access: when an agent runs a statement on a space
 * exposed with `write` access, the backend parks it in the approval queue and
 * emits `mcp-approval-request`. This dialog shows the oldest pending request;
 * the MCP call stays blocked until the user approves, denies, or the backend
 * times the wait out (2 minutes, auto-deny). Unanswered requests rehydrate
 * from the pending list on mount (e.g. after a dev-server reload).
 */
export function McpApprovalDialog() {
  const [queue, setQueue] = useState<McpApprovalRequest[]>([]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    getPendingMcpApprovals()
      .then((pending) => {
        if (!disposed && pending.length > 0) setQueue((q) => [...q, ...pending.filter((p) => !q.some((x) => x.id === p.id))]);
      })
      .catch(() => {});

    listen<McpApprovalRequest>('mcp-approval-request', (event) => {
      setQueue((q) => (q.some((x) => x.id === event.payload.id) ? q : [...q, event.payload]));
    })
      .then((un) => {
        if (disposed) un();
        else unlisten = un;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const respond = async (approved: boolean) => {
    const current = queue[0];
    if (!current) return;
    setQueue((q) => q.slice(1));
    // If this fails the backend keeps the entry until its own timeout fires.
    await respondMcpApproval(current.id, approved).catch(() => {});
  };

  const current = queue[0];
  if (!current) return null;

  return (
    <div className="fixed inset-0 flex items-center justify-center p-4" style={{ zIndex: 10001 }}>
      <div className="bg-black/50 absolute inset-0" />
      <div className="relative bg-[var(--bg-secondary)] p-5 rounded-2xl shadow-2xl w-full max-w-lg animate-fade-in">
        <div className="flex items-start justify-between mb-3">
          <div className="flex items-center gap-2">
            <ShieldAlert className="w-5 h-5 text-[var(--warning, #f5a623)]" />
            <h3 className="text-lg font-semibold">MCP approval requested</h3>
          </div>
          {queue.length > 1 && (
            <span className="text-xs text-[var(--text-secondary)] bg-white/5 rounded px-2 py-0.5">
              +{queue.length - 1} more
            </span>
          )}
          <button
            onClick={() => respond(false)}
            className="text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
            aria-label="Dismiss (deny)"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <p className="text-sm text-[var(--text-secondary)] mb-3">
          An AI agent wants to run a statement on{' '}
          <span className="font-medium text-[var(--text-primary)]">{current.space_name}</span>{' '}
          <span className="text-xs bg-white/5 border border-white/10 rounded px-1.5 py-0.5 uppercase">{current.driver}</span>
          . It runs with the saved connection's own permissions.
        </p>

        <pre className="text-xs bg-white/5 border border-white/10 rounded-lg p-3 overflow-x-auto max-h-48 whitespace-pre-wrap break-words mb-3">
          {current.statement}
        </pre>

        <p className="text-xs text-[var(--text-muted)] mb-4">
          Auto-denies after 2 minutes if unanswered. Every decision is recorded in the audit log.
        </p>

        <div className="flex justify-end gap-2">
          <button
            onClick={() => respond(false)}
            className="px-3 py-1.5 rounded-md text-sm hover:bg-white/5 text-[var(--text-secondary)]"
          >
            Deny
          </button>
          <button
            onClick={() => respond(true)}
            className="px-3 py-1.5 rounded-md text-sm font-medium bg-[var(--accent-color)] text-white hover:opacity-90"
          >
            Approve
          </button>
        </div>
      </div>
    </div>
  );
}
