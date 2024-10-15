import { FILTER_CONDITIONS, FilteringCriteria, FilteringDefaults, FilteringState } from "../filtering";
import { PagingError, PagingState } from "../paging";
import { SortingDefaults, SortingState } from "../sorting";
import { DatasetState, DataType } from "./types";

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
     * Retrieves filtering conditions for a specific data type.
     * @param dataType The data type to get conditions for.
     * @returns An array of FilterCriteria for the specified data type.
     */
    static getFilterConditions(dataType: DataType): FilteringCriteria[] {
        let dt: string;
        switch (dataType) {
            case DataType.String:
                dt = "string";
                break;
            case DataType.Number:
                dt = "number";
                break;
            case DataType.Boolean:
                dt = "boolean";
                break;
            case DataType.Date:
                dt = "date";
                break;
        }
        return FILTER_CONDITIONS[dt];
    }

    /**
     * Gets a list of filtering condition names for a specific data type.
     * @param dataType The data type to get condition names for.
     * @returns An array of condition names as strings.
     */
    static getFilterConditionNames(dataType: DataType): string[] {
        return Object.keys(DataUtil.getFilterConditions(dataType));
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
        const res = [];
        const recordsPerPage = state.itemsPerPage;
        state.details = {
            totalPages: 0,
            totalItems: data.length,
            errorType: PagingError.None
        };
        if (index < 0 || isNaN(index)) {
            state.details.errorType = PagingError.InvalidPageNumber;
            return res;
        }
        if (recordsPerPage <= 0 || isNaN(recordsPerPage)) {
            state.details.errorType = PagingError.InvalidItemsPerPage;
            return res;
        }
        state.details.totalPages = Math.ceil(len / recordsPerPage);
        if (!len) {
            return data;
        }
        if (index >= state.details.totalPages) {
            state.details.errorType = PagingError.InvalidPageNumber;
            return res;
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
    static processDataset<T>(data: T[], state: DatasetState): T[] {
        if (!state) {
            return data;
        }
        if (state.filtering) {
            data = DataUtil.applyFiltering(data, state.filtering);
        }
        if (state.sorting) {
            data = DataUtil.applyOrdering(data, state.sorting);
        }
        if (state.paging) {
            data = DataUtil.applyPagination(data, state.paging);
        }
        return data;
    }
}
