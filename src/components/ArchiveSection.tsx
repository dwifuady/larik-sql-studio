// Archive section button at bottom of sidebar
import { useAppStore } from '../store';
import { useEffect } from 'react';

export function ArchiveSection() {
  const {
    archivedTabsCount,
    loadArchiveCount,
    setArchiveModalOpen,
    activeSpaceId,
  } = useAppStore();

  // Load archive count when active space changes
  useEffect(() => {
    if (activeSpaceId) {
      loadArchiveCount(activeSpaceId);
    }
  }, [activeSpaceId, loadArchiveCount]);

  return (
    <div className="border-t border-white/5 mt-auto">
      <div className="px-1.5 py-1">
        <button
          onClick={() => setArchiveModalOpen(true)}
          className="w-full flex items-center justify-between gap-2 px-2.5 py-1.5 text-xs text-[--text-secondary] hover:bg-white/5 rounded-md transition-colors group"
        >
          <div className="flex items-center gap-2">
            <svg
              className="w-3.5 h-3.5 opacity-60 group-hover:opacity-100"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.5}
                d="M5 8h14M5 8a2 2 0 110-4h14a2 2 0 110 4M5 8v10a2 2 0 002 2h10a2 2 0 002-2V8m-9 4h4"
              />
            </svg>
            <span className="opacity-70 group-hover:opacity-100">Archive</span>
          </div>
          {archivedTabsCount > 0 && (
            <span className="px-1 py-0 text-[9px] font-medium bg-white/10 text-[--text-secondary] rounded">
              {archivedTabsCount}
            </span>
          )}
        </button>
      </div>
    </div>
  );
}
