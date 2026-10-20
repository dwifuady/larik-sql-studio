export type SpaceSwipeDirection = 'next' | 'prev';

export interface SpaceSwipeSample {
  deltaX: number;
  deltaY: number;
  deltaMode: number;
  timeStamp: number;
}

export class SpaceSwipeDetector {
  private accX = 0;
  private accY = 0;
  private lastTime = 0;
  private cooldownUntil = 0;

  constructor(
    private threshold = 120,
    private cooldownMs = 800,
    private idleResetMs = 220,
  ) {}

  feed(sample: SpaceSwipeSample): SpaceSwipeDirection | null {
    if (sample.deltaMode !== 0) {
      this.reset();
      return null;
    }
    if (sample.timeStamp - this.lastTime > this.idleResetMs) {
      this.reset();
    }
    this.lastTime = sample.timeStamp;
    if (sample.timeStamp < this.cooldownUntil) {
      return null;
    }
    if (this.accX !== 0 && Math.sign(sample.deltaX) !== Math.sign(this.accX)) {
      this.reset();
    }
    this.accX += sample.deltaX;
    this.accY += sample.deltaY;
    if (
      Math.abs(this.accX) >= this.threshold &&
      Math.abs(this.accX) >= 2 * Math.abs(this.accY)
    ) {
      const direction: SpaceSwipeDirection = this.accX > 0 ? 'next' : 'prev';
      this.cooldownUntil = sample.timeStamp + this.cooldownMs;
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
