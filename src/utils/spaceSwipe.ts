export type SpaceSwipeDirection = 'next' | 'prev';

export interface SpaceSwipeSample {
  deltaX: number;
  deltaY: number;
  deltaMode: number;
  timeStamp: number;
}

/** Pixel equivalents for line- and page-mode wheel deltas. */
const LINE_HEIGHT_PX = 16;
const PAGE_HEIGHT_PX = 400;

/**
 * Trackpads report pixel deltas (deltaMode 0). Line/page mode deltas (Firefox,
 * some mice) must be converted before they are compared against a pixel
 * threshold.
 */
export function normalizeWheelDelta(delta: number, deltaMode: number): number {
  if (deltaMode === 1) return delta * LINE_HEIGHT_PX;
  if (deltaMode === 2) return delta * PAGE_HEIGHT_PX;
  return delta;
}

export class SpaceSwipeDetector {
  private accX = 0;
  private accY = 0;
  private lastTime = 0;
  /**
   * One switch per gesture. A swipe is not one wheel event: the trackpad streams
   * them while the fingers move, then the OS sends a momentum tail after a short
   * pause. Accumulating alone would switch again on that tail and bounce between
   * two spaces, so the detector latches after a switch and only re-arms across a
   * real pause.
   */
  private armed = true;

  constructor(
    private threshold = 120,
    private idleResetMs = 500,
  ) {}

  feed(sample: SpaceSwipeSample): SpaceSwipeDirection | null {
    const deltaX = normalizeWheelDelta(sample.deltaX, sample.deltaMode);
    const deltaY = normalizeWheelDelta(sample.deltaY, sample.deltaMode);

    if (sample.timeStamp - this.lastTime > this.idleResetMs) {
      // Quiet gap: whatever came before was a separate gesture.
      this.reset();
      this.armed = true;
    }
    this.lastTime = sample.timeStamp;
    if (!this.armed) return null;

    if (this.accX !== 0 && Math.sign(deltaX) !== Math.sign(this.accX)) {
      this.reset();
    }
    this.accX += deltaX;
    this.accY += deltaY;
    if (
      Math.abs(this.accX) >= this.threshold &&
      Math.abs(this.accX) >= 2 * Math.abs(this.accY)
    ) {
      const direction: SpaceSwipeDirection = this.accX > 0 ? 'next' : 'prev';
      this.armed = false;
      this.reset();
      return direction;
    }
    return null;
  }

  reset(): void {
    this.accX = 0;
    this.accY = 0;
  }
}

/** Text widgets treat horizontal wheel input as cursor movement. */
export function isTextEditingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.closest('input, textarea, select, [contenteditable="true"]') !== null;
}

/**
 * True when an ancestor can still scroll horizontally in the swipe direction,
 * i.e. the swipe belongs to that scroller. At a scroll edge there is no room
 * left, so the swipe falls through to space switching (browser-like).
 */
export function hasHorizontalScrollRoom(target: EventTarget | null, deltaX: number): boolean {
  if (!(target instanceof HTMLElement)) return false;
  let el: HTMLElement | null = target;
  while (el) {
    const style = window.getComputedStyle(el);
    if (style.overflowX === 'auto' || style.overflowX === 'scroll') {
      const maxScroll = el.scrollWidth - el.clientWidth;
      if (maxScroll > 1) {
        const room = deltaX > 0 ? maxScroll - el.scrollLeft : el.scrollLeft;
        if (room > 1) return true;
      }
    }
    el = el.parentElement;
  }
  return false;
}

/**
 * Whether a horizontal wheel event inside the swipe zone should be ignored
 * because the element under the cursor handles it itself.
 */
export function swipeBlockedByTarget(target: EventTarget | null, deltaX: number): boolean {
  if (isTextEditingTarget(target)) return true;
  return hasHorizontalScrollRoom(target, deltaX);
}
