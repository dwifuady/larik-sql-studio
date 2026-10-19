import { useEffect, useRef } from 'react';
import { useAppStore } from '../store';
import { SpaceSwipeDetector } from '../utils/spaceSwipe';

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.closest('input, textarea, select, [contenteditable="true"], .monaco-editor') !== null
  );
}

function consumesHorizontalWheel(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.closest('.monaco-editor, .ag-root-wrapper')) return true;
  let el: HTMLElement | null = target;
  while (el) {
    const style = window.getComputedStyle(el);
    if (
      (style.overflowX === 'auto' || style.overflowX === 'scroll') &&
      el.scrollWidth > el.clientWidth + 1
    ) {
      return true;
    }
    el = el.parentElement;
  }
  return false;
}

function isOverlayOpen(): boolean {
  const s = useAppStore.getState();
  return (
    s.commandPaletteOpen ||
    s.shortcutsDialogOpen ||
    s.snippetsDialogOpen ||
    s.settingsDialogOpen ||
    s.createSpaceModalOpen ||
    s.newTabSelectorOpen ||
    s.archiveModalOpen ||
    s.peek.open
  );
}

export function useSpaceSwipe(): void {
  const detectorRef = useRef<SpaceSwipeDetector | null>(null);
  if (!detectorRef.current) {
    detectorRef.current = new SpaceSwipeDetector(100, 800, 220);
  }

  useEffect(() => {
    const onWheel = (e: WheelEvent) => {
      if (!e.cancelable) return;
      if (e.ctrlKey || e.metaKey) return;
      if (isOverlayOpen()) return;
      if (isEditableTarget(e.target)) return;
      if (consumesHorizontalWheel(e.target)) return;

      const direction = detectorRef.current!.feed({
        // Shift+wheel scrolls horizontally on mice/trackpads without horizontal scroll.
        deltaX: e.shiftKey && e.deltaX === 0 ? e.deltaY : e.deltaX,
        deltaY: e.shiftKey && e.deltaX === 0 ? 0 : e.deltaY,
        deltaMode: e.deltaMode,
        timeStamp: e.timeStamp || performance.now(),
      });
      if (!direction) return;

      e.preventDefault();
      void useAppStore.getState().switchSpaceRelative(direction);
    };

    // Mouse back/forward buttons (and some trackpad gestures mapped to them).
    const onMouseDown = (e: MouseEvent) => {
      if (isOverlayOpen()) return;
      if (e.button === 3) {
        e.preventDefault();
        void useAppStore.getState().switchSpaceRelative('next');
      } else if (e.button === 4) {
        e.preventDefault();
        void useAppStore.getState().switchSpaceRelative('prev');
      }
    };

    window.addEventListener('wheel', onWheel, { passive: false });
    window.addEventListener('mousedown', onMouseDown);
    return () => {
      window.removeEventListener('wheel', onWheel);
      window.removeEventListener('mousedown', onMouseDown);
    };
  }, []);
}
