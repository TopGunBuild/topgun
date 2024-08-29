/**
 * An inclusive range of keys that differ between two serialized ordered sets
 * of [`PageRange`].
 */
export class DiffRange<K>
{
    /**
     * The inclusive start & end key bounds of this range to sync.
     */
    constructor(public start: K, public end: K)
    {
    }

    /**
     * Returns true if the range within `self` overlaps any portion of the
     * range within `p`.
     */
    overlaps(p: DiffRange<K>): boolean
    {
        return p.end >= this.start && p.start <= this.end;
    }

    clone(): DiffRange<K>
    {
        return new DiffRange<K>(this.start, this.end);
    }
}
