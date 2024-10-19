import { FilteringDefaults, FilteringState } from "../filtering";
import { PagingState } from "../paging";
import { SortingDefaults, SortingState } from "../sorting";
import { DatasetState } from "./types";

/**
 * Utility class for data processing operations.
 */
export class DataUtil {

    /**
     * Merges default properties into a target object.
     * @param target The target object to merge into.
     * @param defaults The default properties to merge.
     * @returns The merged object.
     */
    static applyDefaults(target: object, defaults: object): object {
        if (!defaults) {
            return target;
        }
        if (!target) {
            target = Object.assign({}, defaults);
            return target;
        }
        Object
            .keys(defaults)
            .forEach((key) => {
                if (target[key] === undefined && defaults[key] !== undefined) {
                    target[key] = defaults[key];
                }
            });
        return target;
    }

    /**
     * Applies sorting to a dataset.
     * @param data The dataset to sort.
     * @param state The sorting state to apply.
     * @returns The sorted dataset.
     */
    static applyOrdering<T>(data: T[], state: SortingState): T[] {
        // set defaults
        DataUtil.applyDefaults(state, SortingDefaults);
        // apply default settings for each sorting expression(if not set)
        return state.algorithm.process(data, state.criteria);
    }

    /**
     * Applies pagination to a dataset.
     * @param data The dataset to paginate.
     * @param state The pagination state to apply.
     * @returns The paginated dataset.
     */
    static applyPagination<T>(data: T[], state: PagingState): T[] {
        if (!state) {
            return data;
        }
        const len = data.length;
        const index = state.currentPage;
        const recordsPerPage = state.itemsPerPage;
        state.details = {
            totalPages: Math.ceil(len / recordsPerPage),
            totalItems: data.length,
            errorType: 0 // Assuming 0 is the value for PagingError.None
        };
        if (!len) {
            return data;
        }
        return data.slice(index * recordsPerPage, (index + 1) * recordsPerPage);
    }

    /**
     * Applies filtering to a dataset.
     * @param data The dataset to filter.
     * @param state The filtering state to apply.
     * @returns The filtered dataset.
     */
    static applyFiltering<T>(data: T[], state: FilteringState): T[] {
        // set defaults
        DataUtil.applyDefaults(state, FilteringDefaults);
        if (!state.algorithm) {
            return data;
        }

        return state.algorithm.process(data, state.tree);
    }

    /**
     * Processes a dataset by applying filtering, ordering, and pagination.
     * @param data The dataset to process.
     * @param state The dataset state containing filtering, ordering, and pagination information.
     * @returns The processed dataset.
     */
    static processDataset<T>(data: T[], state: DatasetState): {rows: T[], total: number} {
        let total = data.length;
        let rows = [...data];
        if (!state) {
            return {rows, total};
        }
        if (state.filtering) {
            rows = DataUtil.applyFiltering(rows, state.filtering);
            total = rows.length;
        }
        if (state.sorting) {
            rows = DataUtil.applyOrdering(rows, state.sorting);
        }
        if (state.paging) {
            rows = DataUtil.applyPagination(rows, state.paging);
        }
        return {rows, total};
    }
}
