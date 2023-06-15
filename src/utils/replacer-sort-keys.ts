export const replacerSortKeys = (key: string, value: unknown) =>
    value instanceof Object && !(value instanceof Array) ?
        Object.keys(value)
            .sort()
            .reduce((sorted, key) =>
            {
                sorted[key] = value[key];
                return sorted
            }, {}) :
        value;
