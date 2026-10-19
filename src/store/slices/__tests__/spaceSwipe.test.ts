import { describe, it, expect, vi, beforeEach } from 'vitest';
import { create } from 'zustand';
import { createSpacesSlice } from '../spacesSlice';
import { SpaceSwipeDetector, type SpaceSwipeDirection } from '../../../utils/spaceSwipe';

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

describe('SpaceSwipeDetector', () => {
  it('fires once per accumulated swipe gesture', () => {
    const detector = new SpaceSwipeDetector(120, 800, 220);
    let t = 1000;
    let first: SpaceSwipeDirection | null = null;
    for (let i = 0; i < 6; i++) {
      t += 16;
      first = feedAt(detector, 30, 0, t) ?? first;
    }
    expect(first).toBe('next');

    // A second gesture inside the cooldown window is swallowed.
    let second: SpaceSwipeDirection | null = null;
    for (let i = 0; i < 6; i++) {
      t += 16;
      second = feedAt(detector, 30, 0, t) ?? second;
    }
    expect(second).toBeNull();

    // After the cooldown a fresh gesture fires again.
    t += 900;
    let third: SpaceSwipeDirection | null = null;
    for (let i = 0; i < 6; i++) {
      t += 16;
      third = feedAt(detector, 30, 0, t) ?? third;
    }
    expect(third).toBe('next');
  });

  it('ignores mostly-vertical scrolling and resets on direction flips', () => {
    const detector = new SpaceSwipeDetector(120, 0, 220);
    let t = 100;
    let direction: SpaceSwipeDirection | null = null;
    for (let i = 0; i < 10; i++) {
      t += 16;
      direction = feedAt(detector, 10, 40, t) ?? direction;
    }
    expect(direction).toBeNull();

    // A direction flip discards the stale positive accumulation and can fire on its own.
    t += 16;
    expect(feedAt(detector, -130, 0, t)).toBe('prev');
  });
});
