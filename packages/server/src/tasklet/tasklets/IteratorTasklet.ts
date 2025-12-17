/**
 * IteratorTasklet — Base class for tasklets that iterate over collections.
 *
 * Provides cooperative iteration with time-budgeted chunks.
 * Subclasses implement processItem() to handle each item.
 */

import { Tasklet, ProgressState } from '../TaskletScheduler';

/**
 * Configuration for iterator tasklets.
 */
export interface IteratorTaskletConfig {
    /** Time budget per iteration in ms (default: 5) */
    timeBudgetMs?: number;

    /** Maximum items to process per iteration (default: 1000) */
    maxItemsPerIteration?: number;
}

const DEFAULT_CONFIG: Required<IteratorTaskletConfig> = {
    timeBudgetMs: 5,
    maxItemsPerIteration: 1000,
};

/**
 * Abstract base class for iterator-based tasklets.
 *
 * Usage:
 * ```typescript
 * class MyTasklet extends IteratorTasklet<[string, Record], Record[]> {
 *     constructor(map: Map<string, Record>) {
 *         super('my-tasklet', map.entries());
 *     }
 *
 *     protected processItem([key, record]: [string, Record]): void {
 *         if (matchesCriteria(record)) {
 *             this.results.push(record);
 *         }
 *     }
 *
 *     getResult(): Record[] {
 *         return this.results;
 *     }
 * }
 * ```
 */
export abstract class IteratorTasklet<TItem, TResult> implements Tasklet<TResult> {
    abstract readonly name: string;

    protected readonly config: Required<IteratorTaskletConfig>;
    protected readonly iterator: Iterator<TItem>;
    protected itemsProcessed = 0;
    protected isDone = false;

    constructor(
        iterator: Iterator<TItem>,
        config?: IteratorTaskletConfig
    ) {
        this.iterator = iterator;
        this.config = { ...DEFAULT_CONFIG, ...config };
    }

    /**
     * Process a single item from the iterator.
     * Override this in subclasses.
     */
    protected abstract processItem(item: TItem): void;

    /**
     * Get the final result after iteration completes.
     */
    abstract getResult(): TResult;

    /**
     * Execute one chunk of iteration.
     */
    call(): ProgressState {
        if (this.isDone) {
            return 'DONE';
        }

        const deadline = Date.now() + this.config.timeBudgetMs;
        let processedThisIteration = 0;

        while (
            Date.now() < deadline &&
            processedThisIteration < this.config.maxItemsPerIteration
        ) {
            const { value, done } = this.iterator.next();

            if (done) {
                this.isDone = true;
                return 'DONE';
            }

            this.processItem(value);
            this.itemsProcessed++;
            processedThisIteration++;
        }

        return 'MADE_PROGRESS';
    }

    /**
     * Called when tasklet is cancelled.
     */
    onCancel(): void {
        // Subclasses can override for cleanup
    }

    /**
     * Get number of items processed so far.
     */
    get processed(): number {
        return this.itemsProcessed;
    }
}

/**
 * Simple iterator tasklet that collects items matching a predicate.
 */
export class FilterTasklet<T> extends IteratorTasklet<T, T[]> {
    readonly name: string;
    protected readonly results: T[] = [];
    private readonly predicate: (item: T) => boolean;

    constructor(
        name: string,
        iterator: Iterator<T>,
        predicate: (item: T) => boolean,
        config?: IteratorTaskletConfig
    ) {
        super(iterator, config);
        this.name = name;
        this.predicate = predicate;
    }

    protected processItem(item: T): void {
        if (this.predicate(item)) {
            this.results.push(item);
        }
    }

    getResult(): T[] {
        return this.results;
    }
}

/**
 * Iterator tasklet that transforms items.
 */
export class MapTasklet<TIn, TOut> extends IteratorTasklet<TIn, TOut[]> {
    readonly name: string;
    protected readonly results: TOut[] = [];
    private readonly mapper: (item: TIn) => TOut;

    constructor(
        name: string,
        iterator: Iterator<TIn>,
        mapper: (item: TIn) => TOut,
        config?: IteratorTaskletConfig
    ) {
        super(iterator, config);
        this.name = name;
        this.mapper = mapper;
    }

    protected processItem(item: TIn): void {
        this.results.push(this.mapper(item));
    }

    getResult(): TOut[] {
        return this.results;
    }
}

/**
 * Iterator tasklet that applies a function to each item (side effects).
 */
export class ForEachTasklet<T> extends IteratorTasklet<T, number> {
    readonly name: string;
    private readonly action: (item: T) => void;

    constructor(
        name: string,
        iterator: Iterator<T>,
        action: (item: T) => void,
        config?: IteratorTaskletConfig
    ) {
        super(iterator, config);
        this.name = name;
        this.action = action;
    }

    protected processItem(item: T): void {
        this.action(item);
    }

    getResult(): number {
        return this.itemsProcessed;
    }
}

/**
 * Iterator tasklet that reduces items to a single value.
 */
export class ReduceTasklet<T, TAccum> extends IteratorTasklet<T, TAccum> {
    readonly name: string;
    private accumulator: TAccum;
    private readonly reducer: (acc: TAccum, item: T) => TAccum;

    constructor(
        name: string,
        iterator: Iterator<T>,
        initialValue: TAccum,
        reducer: (acc: TAccum, item: T) => TAccum,
        config?: IteratorTaskletConfig
    ) {
        super(iterator, config);
        this.name = name;
        this.accumulator = initialValue;
        this.reducer = reducer;
    }

    protected processItem(item: T): void {
        this.accumulator = this.reducer(this.accumulator, item);
    }

    getResult(): TAccum {
        return this.accumulator;
    }
}
