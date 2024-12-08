/**
 * Helper function to apply ignore case to a string.
 * @param input - The string to apply ignore case to.
 * @param ignoreCase - Whether to ignore case.
 * @returns The string with ignore case applied.
 */
export function applyIgnoreCase(input: string, ignoreCase: boolean): string {
    // Ensure input is a string, defaulting to empty string if falsy
    const safeInput = input || "";
    
    // Convert to lowercase if ignoreCase is true, otherwise return as-is
    return ignoreCase ? safeInput.toLowerCase() : safeInput;
}

/**
 * Helper function to strip '%' characters from the beginning and end of a string.
 * These characters are used in SQL LIKE queries but not needed for JavaScript string operations.
 * @param input - The string to strip '%' characters from.
 * @returns The string with '%' characters removed from start and end.
 */
export function stripWildcards(input: string): string {
    // Return empty string if input is falsy
    if (!input) {
        return '';
    }

    // Remove leading and trailing '%' characters
    return input.replace(/^%+|%+$/g, '');
}


/**
 * Interface representing date components
 */
export interface DateComponents {
    year: number | null;
    month: number | null;
    day: number | null;
    hours: number | null;
    minutes: number | null;
    seconds: number | null;
    milliseconds: number | null;
}

/**
 * Helper function to extract date components from a date.
 * @param date - The date to extract components from.
 * @param format - The format to extract components from.
 * @returns The components of the date.
 */
export function extractDateComponents(date: Date, format?: string): DateComponents {
    const components: DateComponents = {
        year: null,
        month: null,
        day: null,
        hours: null,
        minutes: null,
        seconds: null,
        milliseconds: null
    };

    if (!date || !format) {
        return components;
    }

    const formatMap = {
        'y': () => components.year = date.getFullYear(),
        'M': () => components.month = date.getMonth(),
        'd': () => components.day = date.getDate(),
        'h': () => components.hours = date.getHours(),
        'm': () => components.minutes = date.getMinutes(),
        's': () => components.seconds = date.getSeconds(),
        'f': () => components.milliseconds = date.getMilliseconds()
    };

    for (const [char, setter] of Object.entries(formatMap)) {
        if (format.includes(char)) {
            setter();
        }
    }

    return components;
}
