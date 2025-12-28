/**
 * Fixed-size circular buffer with sequence numbers.
 * Older entries are overwritten when capacity is reached.
 *
 * @template T - Type of items stored in the buffer
 */
export class Ringbuffer<T> {
  private buffer: (T | undefined)[];
  private readonly _capacity: number;
  private headSequence: bigint = 0n; // Oldest available sequence
  private tailSequence: bigint = 0n; // Next sequence to write

  constructor(capacity: number) {
    if (capacity < 1) {
      throw new Error('Capacity must be >= 1');
    }
    this._capacity = capacity;
    this.buffer = new Array(capacity);
  }

  /**
   * Add item to buffer, returns sequence number.
   */
  add(item: T): bigint {
    const sequence = this.tailSequence;
    const index = Number(sequence % BigInt(this._capacity));

    this.buffer[index] = item;
    this.tailSequence++;

    // Advance head if we overwrote oldest
    if (this.tailSequence - this.headSequence > this._capacity) {
      this.headSequence = this.tailSequence - BigInt(this._capacity);
    }

    return sequence;
  }

  /**
   * Read item at sequence.
   * Returns undefined if sequence is out of range.
   */
  read(sequence: bigint): T | undefined {
    if (sequence < this.headSequence || sequence >= this.tailSequence) {
      return undefined;
    }
    const index = Number(sequence % BigInt(this._capacity));
    return this.buffer[index];
  }

  /**
   * Read range of items (inclusive).
   * Automatically clamps to available range.
   */
  readRange(startSeq: bigint, endSeq: bigint): T[] {
    const items: T[] = [];
    const actualStart = startSeq < this.headSequence ? this.headSequence : startSeq;
    const actualEnd = endSeq >= this.tailSequence ? this.tailSequence - 1n : endSeq;

    for (let seq = actualStart; seq <= actualEnd; seq++) {
      const item = this.read(seq);
      if (item !== undefined) {
        items.push(item);
      }
    }

    return items;
  }

  /**
   * Read from sequence with limit.
   */
  readFrom(startSeq: bigint, limit: number = 100): T[] {
    const endSeq = startSeq + BigInt(limit) - 1n;
    return this.readRange(startSeq, endSeq);
  }

  /**
   * Get the oldest available sequence number.
   */
  getHeadSequence(): bigint {
    return this.headSequence;
  }

  /**
   * Get the next sequence number to be written.
   */
  getTailSequence(): bigint {
    return this.tailSequence;
  }

  /**
   * Get the number of items currently in the buffer.
   */
  size(): number {
    return Number(this.tailSequence - this.headSequence);
  }

  /**
   * Get the maximum capacity of the buffer.
   */
  getCapacity(): number {
    return this._capacity;
  }

  /**
   * Clear all items from the buffer.
   */
  clear(): void {
    this.buffer = new Array(this._capacity);
    this.headSequence = 0n;
    this.tailSequence = 0n;
  }

  /**
   * Check if a sequence is available in the buffer.
   */
  isAvailable(sequence: bigint): boolean {
    return sequence >= this.headSequence && sequence < this.tailSequence;
  }

  /**
   * Get remaining capacity before oldest entries are overwritten.
   */
  remainingCapacity(): number {
    return this._capacity - this.size();
  }
}
