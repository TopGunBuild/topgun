/**
 * Creates a duplicate-free version of an array based on an iteratee function.
 * The iteratee function determines the uniqueness criteria for each element.
 * 
 * @param array The array to process
 * @param iteratee Function that returns the key to determine uniqueness
 * @returns A new array with unique elements
 * 
 * @example
 * const array = [{id: 1, name: 'a'}, {id: 1, name: 'b'}, {id: 2, name: 'c'}];
 * uniqBy(array, item => item.id); // [{id: 1, name: 'a'}, {id: 2, name: 'c'}]
 */
export function uniqBy<T>(array: T[], iteratee: (item: T) => string | number): T[] {
    const seen = new Set<string | number>();
    return array.filter(item => {
        const key = iteratee(item);
        if (seen.has(key)) {
            return false;
        }
        seen.add(key);
        return true;
    });
}
