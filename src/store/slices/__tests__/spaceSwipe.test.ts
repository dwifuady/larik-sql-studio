import { describe, it, expect, vi, beforeEach } from 'vitest';
import { create } from 'zustand';
import { createSpacesSlice } from '../spacesSlice';
import {
  SpaceSwipeDetector,
  swipeBlockedByTarget,
  type SpaceSwipeDirection,
} from '../../../utils/spaceSwipe';

vi.mock('../../../api', () => ({
  getSpaces: vi.fn(),
  getAppSettings: vi.fn(),
  createSpace: vi.fn(),
  updateSpace: vi.fn(),
  deleteSpace: vi.fn(),
  reorderSpaces: vi.fn(),
  connectToSpace: vi.fn(),
  disconnectFromSpace: vi.fn(),
  getSpaceConnectionStatus: vi.fn(),
  getSpaceDatabasesWithAccess: vi.fn(),
  testConnection: vi.fn(),
}));

const useTestStore = create<any>((set, get, storeApi) => ({
  ...createSpacesSlice(set, get, storeApi),
  tabs: [],
  folders: [],
  loadTabs: vi.fn(),
  loadFolders: vi.fn(),
  refreshSpaceConnectionStatus: vi.fn(),
  loadSpaceDatabases: vi.fn(),
  loadSchema: vi.fn(),
  saveAppSettings: vi.fn(),
  isConnected: () => false,
  closePopoutsForSpace: vi.fn(),
}));

const makeSpaces = (n: number) =>
  Array.from({ length: n }, (_, i) => ({
    id: `space-${i + 1}`,
    name: `Space ${i + 1}`,
    color: '#0078d4',
    sort_order: i,
    created_at: '',
    updated_at: '',
  }));

function feedAt(detector: SpaceSwipeDetector, deltaX: number, deltaY: number, t: number) {
  return detector.feed({ deltaX, deltaY, deltaMode: 0, timeStamp: t });
}

/** Feed a burst of events `gapMs` apart and collect whatever the detector fires. */
function feedBurst(
  detector: SpaceSwipeDetector,
  options: { count: number; deltaX: number; deltaY?: number; gapMs?: number; from: number },
): { fired: SpaceSwipeDirection[]; t: number } {
  let t = options.from;
  const fired: SpaceSwipeDirection[] = [];
  for (let i = 0; i < options.count; i++) {
    t += options.gapMs ?? 16;
    const direction = feedAt(detector, options.deltaX, options.deltaY ?? 0, t);
    if (direction) fired.push(direction);
  }
  return { fired, t };
}

describe('spacesSlice.switchSpaceRelative', () => {
  beforeEach(() => {
    useTestStore.setState({
      spaces: makeSpaces(3),
      activeSpaceId: 'space-1',
      tabs: [],
      folders: [],
      spaceConnectionStatus: null,
    });
    vi.clearAllMocks();
  });

  it('moves to the next space with wrap-around', async () => {
    await useTestStore.getState().switchSpaceRelative('next');
    expect(useTestStore.getState().activeSpaceId).toBe('space-2');

    await useTestStore.getState().switchSpaceRelative('next');
    expect(useTestStore.getState().activeSpaceId).toBe('space-3');

    await useTestStore.getState().switchSpaceRelative('next');
    expect(useTestStore.getState().activeSpaceId).toBe('space-1');
  });

  it('moves to the previous space with wrap-around', async () => {
    await useTestStore.getState().switchSpaceRelative('prev');
    expect(useTestStore.getState().activeSpaceId).toBe('space-3');
  });

  it('does nothing with fewer than two spaces', async () => {
    useTestStore.setState({ spaces: makeSpaces(1), activeSpaceId: 'space-1' });
    await useTestStore.getState().switchSpaceRelative('next');
    expect(useTestStore.getState().activeSpaceId).toBe('space-1');
    expect(useTestStore.getState().loadTabs).not.toHaveBeenCalled();
  });
});

describe('SpaceSwipeDetector: one switch per gesture', () => {
  it('fires once when the gesture keeps streaming', () => {
    const detector = new SpaceSwipeDetector(120, 250);
    const first = feedBurst(detector, { count: 6, deltaX: 30, from: 1000 });
    expect(first.fired).toEqual(['next']);

    // Same gesture, more events: latched.
    const second = feedBurst(detector, { count: 6, deltaX: 30, from: first.t });
    expect(second.fired).toEqual([]);

    // A real pause starts a new gesture.
    const third = feedBurst(detector, { count: 6, deltaX: 30, from: second.t + 400 });
    expect(third.fired).toEqual(['next']);
  });

  it('fires at most once for a long continuous swipe', () => {
    const detector = new SpaceSwipeDetector(80, 500);
    // ~5s of uninterrupted two-finger swipe at 60Hz.
    const { fired } = feedBurst(detector, { count: 300, deltaX: -20, from: 1000 });
    expect(fired).toEqual(['prev']);
  });

  it('does not bounce on a slow swipe with pauses inside the gesture', () => {
    const detector = new SpaceSwipeDetector(80, 500);
    let t = 1000;
    const fired: SpaceSwipeDirection[] = [];
    for (let i = 0; i < 120; i++) {
      t += i % 15 === 0 ? 200 : 16;
      const direction = feedAt(detector, -12, 1, t);
      if (direction) fired.push(direction);
    }
    expect(fired).toEqual(['prev']);
  });

  it('ignores the momentum tail that follows the fingers', () => {
    // A swipe arrives in two bursts: the finger movement, then the OS fling,
    // separated by a pause of a few hundred ms. That pause is not a new gesture.
    const detector = new SpaceSwipeDetector(80, 500);
    const movement = feedBurst(detector, { count: 5, deltaX: -20, from: 1000 });
    expect(movement.fired).toEqual(['prev']);

    const fling = feedBurst(detector, { count: 40, deltaX: -20, from: movement.t + 280 });
    expect(fling.fired).toEqual([]);
  });

  it('switches again for a new swipe after a real pause', () => {
    const detector = new SpaceSwipeDetector(80, 500);
    const first = feedBurst(detector, { count: 5, deltaX: -20, from: 1000 });
    const second = feedBurst(detector, { count: 5, deltaX: 20, from: first.t + 700 });
    expect([...first.fired, ...second.fired]).toEqual(['prev', 'next']);
  });

  it('ignores mostly-vertical scrolling and resets on direction flips', () => {
    const detector = new SpaceSwipeDetector(120, 250);
    const vertical = feedBurst(detector, { count: 10, deltaX: 10, deltaY: 40, from: 100 });
    expect(vertical.fired).toEqual([]);

    // A direction flip discards the stale positive accumulation and can fire on its own.
    expect(feedAt(detector, -130, 0, vertical.t + 16)).toBe('prev');
  });
});

describe('SpaceSwipeDetector: delta normalization', () => {
  it('converts line-mode deltas to pixels before thresholding', () => {
    const detector = new SpaceSwipeDetector(120, 1000);
    // 10 lines of horizontal scroll is 160px, above the 120px threshold.
    expect(detector.feed({ deltaX: -10, deltaY: 0, deltaMode: 1, timeStamp: 10 })).toBe('prev');
  });

  it('converts page-mode deltas to pixels before thresholding', () => {
    const detector = new SpaceSwipeDetector(120, 1000);
    expect(detector.feed({ deltaX: -1, deltaY: 0, deltaMode: 2, timeStamp: 10 })).toBe('prev');
  });

  it('keeps pixel deltas untouched', () => {
    const detector = new SpaceSwipeDetector(120, 1000);
    expect(feedAt(detector, -60, 0, 10)).toBeNull();
    expect(feedAt(detector, -60, 0, 20)).toBe('prev');
  });

  it('accepts non-pixel modes instead of discarding them', () => {
    const detector = new SpaceSwipeDetector(120, 1000);
    expect(detector.feed({ deltaX: 0, deltaY: 0, deltaMode: 1, timeStamp: 5 })).toBeNull();
    expect(detector.feed({ deltaX: -20, deltaY: 0, deltaMode: 1, timeStamp: 15 })).toBe('prev');
  });
});

describe('swipeBlockedByTarget', () => {
  const scrollable = (
    el: HTMLElement,
    { scrollWidth, clientWidth, scrollLeft = 0 }: { scrollWidth: number; clientWidth: number; scrollLeft?: number },
  ) => {
    el.style.overflowX = 'auto';
    Object.defineProperty(el, 'scrollWidth', { value: scrollWidth, configurable: true });
    Object.defineProperty(el, 'clientWidth', { value: clientWidth, configurable: true });
    el.scrollLeft = scrollLeft;
    return el;
  };

  it('lets plain sidebar content take the swipe', () => {
    const row = document.createElement('div');
    document.body.appendChild(row);

    expect(swipeBlockedByTarget(row, -120)).toBe(false);

    row.remove();
  });

  it('blocks swipes that start in a text field', () => {
    const input = document.createElement('input');
    document.body.appendChild(input);

    expect(swipeBlockedByTarget(input, -120)).toBe(true);

    input.remove();
  });

  it('blocks swipes a horizontally scrollable list can still consume', () => {
    const scroller = scrollable(document.createElement('div'), {
      scrollWidth: 600,
      clientWidth: 200,
      scrollLeft: 100,
    });
    const child = document.createElement('span');
    scroller.appendChild(child);
    document.body.appendChild(scroller);

    expect(swipeBlockedByTarget(child, -120)).toBe(true);

    scroller.remove();
  });

  it('falls through at the scroll edge so the swipe still switches spaces', () => {
    const scroller = scrollable(document.createElement('div'), {
      scrollWidth: 600,
      clientWidth: 200,
      scrollLeft: 0,
    });
    const child = document.createElement('span');
    scroller.appendChild(child);
    document.body.appendChild(scroller);

    // Nothing left to scroll towards the start, so the space switch wins.
    expect(swipeBlockedByTarget(child, -120)).toBe(false);
    // Scrolling towards the end is still the scroller's business.
    expect(swipeBlockedByTarget(child, 120)).toBe(true);

    scroller.remove();
  });
});
