export const sortKeys = (key: string, value: any) =>
    value instanceof Object && !(value instanceof Array) ?
        Object.keys(value)
            .sort()
            .reduce((sorted, key) =>
            {
                sorted[key] = value[key];
                return sorted
            }, {}) :
        value;
