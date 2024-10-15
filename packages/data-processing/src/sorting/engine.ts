import { SortDirection } from "@topgunbuild/types";
import { SortingCriteria, SortingImplementation, SortingState } from "./types";

/**
 * Class for sorting algorithm.
 */
export class DataSortingEngine implements SortingImplementation {
    /**
     * Arranges the items based on the criteria.
     * @param items The items to arrange.
     * @param criteria The criteria to use for arranging.
     * @returns The arranged items.
     */
    process(items: any[], criteria: SortingCriteria[]): any[] {
        return this.applySortingRecursively(items, criteria);
    }

    /**
     * Compares two elements.
     * @param first The first element to compare.
     * @param second The second element to compare.
     * @returns The comparison result.
     */
    compareElements(first: any, second: any): number {
        const isFirstNull = (first === null || first === undefined);
        const isSecondNull = (second === null || second === undefined);

        if (isFirstNull && isSecondNull) return 0;
        if (isFirstNull) return -1;
        if (isSecondNull) return 1;

        return first > second ? 1 : first < second ? -1 : 0;
    }

    /**
     * Compares two attributes.
     * @param item1 The first item to compare.
     * @param item2 The second item to compare.
     * @param attribute The attribute to compare.
     * @param multiplier The multiplier to use for the comparison.
     * @param caseSensitive Whether the comparison should be case-sensitive.
     * @returns The comparison result.
     */
    protected compareAttributes(
        item1: object,
        item2: object,
        attribute: string,
        multiplier: number,
        caseSensitive: boolean
    ): number {
        let value1 = item1[attribute];
        let value2 = item2[attribute];

        if (!caseSensitive) {
            value1 = value1 && typeof value1 === 'string' ? value1.toLowerCase() : value1;
            value2 = value2 && typeof value2 === 'string' ? value2.toLowerCase() : value2;
        }

        return multiplier * this.compareElements(value1, value2);
    }

    /**
     * Sorts the array using the provided comparator.
     * @param items The items to sort.
     * @param comparator The comparator to use for sorting.
     * @returns The sorted items.
     */
    protected sortArray<T>(items: T[], comparator?: (a: T, b: T) => number): T[] {
        return items.sort(comparator);
    }

    /**
     * Gets the items with the same value.
     * @param items The items to get the items with the same value from.
     * @param startIndex The index to start the search from.
     * @param criteria The criteria to use for getting the items with the same value.
     * @returns The items with the same value.
     */
    private getItemsWithSameValue<T>(items: T[], startIndex: number, criteria: SortingCriteria): T[] {
        const result = [];
        const attribute = criteria.key;
        const referenceValue = items[startIndex][attribute];

        for (let i = startIndex; i < items.length; i++) {
            if (items[i][attribute] === referenceValue) {
                result.push(items[i]);
            } else {
                break;
            }
        }

        return result;
    }

    /**
     * Orders the items by the specified attribute.
     * @param items The items to order.
     * @param criteria The criteria to use for ordering.
     * @returns The ordered items.
     */
    private sortByAttribute<T>(items: T[], criteria: SortingCriteria): T[] {
        const attribute = criteria.key;
        const caseSensitive = criteria.caseSensitive ?? false;
        const multiplier = (criteria.direction === SortDirection.DESC ? -1 : 1);

        const comparator = (item1: any, item2: any): number => {
            return this.compareAttributes(item1, item2, attribute, multiplier, caseSensitive);
        };

        return this.sortArray(items, comparator);
    }

    /**
     * Applies the sorting recursively.
     * @param items The items to sort.
     * @param criteria The criteria to use for sorting.
     * @param criteriaIndex The index of the criteria to use for sorting.
     * @returns The sorted items.
     */
    private applySortingRecursively<T>(
        items: T[],
        criteria: SortingCriteria[],
        criteriaIndex: number = 0
    ): T[] {
        if (criteriaIndex >= criteria.length || items.length <= 1) {
            return items;
        }

        const currentCriteria = criteria[criteriaIndex];
        items = this.sortByAttribute(items, currentCriteria);

        if (criteriaIndex === criteria.length - 1) {
            return items;
        }

        // Handle multiple ordering criteria
        for (let i = 0; i < items.length; i++) {
            const sameValueItems = this.getItemsWithSameValue(items, i, currentCriteria);
            if (sameValueItems.length > 1) {
                const orderedSubset = this.applySortingRecursively(sameValueItems, criteria, criteriaIndex + 1);
                items.splice(i, sameValueItems.length, ...orderedSubset);
            }
            i += sameValueItems.length - 1;
        }

        return items;
    }
}

/**
 * Default sorting algorithm.
 */
export const SortingDefaults: SortingState = {
    criteria: [],
    algorithm: new DataSortingEngine()
};

