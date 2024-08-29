import { ConsoleLogger } from '@topgunbuild/logger';
import { DiffRange } from './diff-range';
import { HasherInput } from '../digest';

const logger = new ConsoleLogger();

/**
 * Helper to construct an ordered list of non-overlapping DiffRange intervals.
 */
export class RangeList<K extends HasherInput>
{
    private readonly syncRanges: DiffRange<K>[];

    constructor()
    {
        this.syncRanges = [];
    }

    /**
     * Insert the inclusive interval [start, end] into the list.
     */
    insert(start: K, end: K): void
    {
        if (start > end)
        {
            throw new Error('Start must be less than or equal to end');
        }
        this.syncRanges.push(new DiffRange(start, end));
    }

    /**
     * Consume self and return the deduplicated/merged list of intervals
     * ordered by range start.
     */
    intoVec(): DiffRange<K>[]
    {
        this.syncRanges.sort((a, b) => this.compare(a.start, b.start));
        mergeOverlapping(this.syncRanges);

        // Check invariants in development builds.
        // if (process.env.NODE_ENV !== 'production')
        // {
        //   for (let i = 0; i < this.syncRanges.length - 1; i++)
        //   {
        //     const current = this.syncRanges[i];
        //     const next    = this.syncRanges[i + 1];
        //
        //     // Invariant: non-overlapping ranges
        //     if (this.overlaps(current, next))
        //     {
        //       throw new Error('Overlapping ranges detected');
        //     }
        //
        //     // Invariant: end bound is always gte than start bound
        //     if (this.compare(current.start, current.end) > 0 || this.compare(next.start, next.end) > 0)
        //     {
        //       throw new Error('Diff range contains inverted bounds');
        //     }
        //   }
        // }

        return this.syncRanges;
    }

    private compare(a: K, b: K): number
    {
        if (a < b) return -1;
        if (a > b) return 1;
        return 0;
    }

    // private overlaps(a: DiffRange<K>, b: DiffRange<K>): boolean
    // {
    //   return this.compare(a.start, b.end) <= 0 && this.compare(b.start, a.end) <= 0;
    // }
}

/**
 * Perform an in-place merge and deduplication of overlapping intervals.
 *
 * Assumes the intervals within `source` are sorted by the start value.
 */
export function mergeOverlapping<K>(source: DiffRange<K>[]): void
{
    const nRanges   = source.length;
    const rangeIter = source.splice(0, nRanges);

    // Pre-allocate the ranges array to hold all the elements, pessimistically
    // expecting them to not contain overlapping regions.
    source.length = 0;
    source.push(...new Array(nRanges));

    // Place the first range into the merged output array.
    const firstRange = rangeIter.shift();
    if (!firstRange)
    {
        return;
    }
    source[0] = firstRange;

    let sourceIndex = 0;

    for (const range of rangeIter)
    {
        const last = source[sourceIndex];

        // Invariant: ranges are sorted by range start.
        if (range.start >= last.start)
        {
            logger.warn('Ranges must be sorted by start');
        }

        // Check if this range falls entirely within the existing range.
        if (range.end <= last.end)
        {
            // Skip this range that is a subset of the existing range.
            continue;
        }

        // Check for overlap across the end ranges (inclusive).
        if (range.start <= last.end)
        {
            // These two ranges overlap - extend the range in "last" to cover
            // both.
            last.end = range.end;
        }
        else
        {
            sourceIndex++;
            source[sourceIndex] = range;
        }
    }

    // Trim any unused pre-allocated space
    source.length = sourceIndex + 1;
}

