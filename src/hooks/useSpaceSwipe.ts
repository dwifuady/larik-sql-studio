import { useEffect, useRef, type RefObject } from 'react';
import { useAppStore } from '../store';
import { SpaceSwipeDetector, normalizeWheelDelta, swipeBlockedByTarget } from '../utils/spaceSwipe';

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
    s.referenceEditor.open ||
    s.peek.open
  );
}

/**
 * Swipe left/right on the left sidebar to switch spaces.
 *
 * The listener lives on the sidebar element itself rather than on `window`:
 * that scopes the gesture to the pane the user pointed at (the editor, grids and
 * preview panes keep their horizontal input) and it also means the events of a
 * gesture that outlives a space switch simply stop arriving, because the sidebar
 * subtree they were dispatched against was replaced.
 */
export function useSpaceSwipe(zoneRef: RefObject<HTMLElement | null>): void {
  const detectorRef = useRef<SpaceSwipeDetector | null>(null);
  if (!detectorRef.current) {
    detectorRef.current = new SpaceSwipeDetector(80);
  }

  useEffect(() => {
    const zone = zoneRef.current;
    const detector = detectorRef.current;
    if (!zone || !detector) return;

    const onWheel = (e: WheelEvent) => {
      // Ctrl/Cmd + wheel is pinch-zoom, never a swipe.
      if (e.ctrlKey || e.metaKey) return;
      if (isOverlayOpen()) return;

      // Shift+wheel scrolls horizontally on mice/trackpads without horizontal scroll.
      const shifted = e.shiftKey && e.deltaX === 0;
      const deltaX = normalizeWheelDelta(shifted ? e.deltaY : e.deltaX, e.deltaMode);
      const deltaY = shifted ? 0 : normalizeWheelDelta(e.deltaY, e.deltaMode);

      if (swipeBlockedByTarget(e.target, deltaX)) return;

      // Deltas are already in pixels, so the detector is fed deltaMode 0. Track
      // the gesture even when the event is not cancelable: Chromium stops marking
      // wheel events cancelable once it owns the scroll.
      const direction = detector.feed({
        deltaX,
        deltaY,
        deltaMode: 0,
        timeStamp: e.timeStamp || performance.now(),
      });
      if (!direction) return;

      if (e.cancelable) e.preventDefault();
      // Keep the consumed swipe from reaching pane-level handlers as well.
      e.stopPropagation();
      void useAppStore.getState().switchSpaceRelative(direction);
    };

    zone.addEventListener('wheel', onWheel, { passive: false });
    return () => zone.removeEventListener('wheel', onWheel);
  }, [zoneRef]);
}
