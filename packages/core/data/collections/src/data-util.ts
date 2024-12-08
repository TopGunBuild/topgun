import { Identifiable } from "@topgunbuild/models";
import { FilterEngine, FilterDefaults, FilterState, FilterGroup } from "./filter";
import { PaginationState } from "./pagination";
import { SortDefaults, SortState } from "./sort";
import { ChangeType, DataChange, DatasetState } from "./types";

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
    static applySorting<T>(data: T[], state: SortState): T[] {
        // set defaults
        DataUtil.applyDefaults(state, SortDefaults);
        // apply default settings for each sorting expression(if not set)
        return state.engine.process(data, state.options);
    }

    /**
     * Applies pagination to a dataset.
     * @param data The dataset to paginate.
     * @param state The pagination state to apply.
     * @returns The paginated dataset.
     */
    static applyPagination<T>(data: T[], state: PaginationState): T[] {
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
    static applyFiltering<T>(data: T[], state: FilterState): T[] {
        // set defaults
        DataUtil.applyDefaults(state, FilterDefaults);
        if (!state.engine) {
            return data;
        }

        return state.engine.process(data, state.options);
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
        if (state.filter) {
            rows = DataUtil.applyFiltering(rows, state.filter);
            total = rows.length;
        }
        if (state.sort) {
            rows = DataUtil.applySorting(rows, state.sort);
        }
        if (state.page) {
            rows = DataUtil.applyPagination(rows, state.page);
        }
        return {rows, total};
    }

    /**
     * Process changes between old and new datasets
     * @param oldData The original dataset
     * @param newData The new dataset to compare against
     * @returns Array of changes with their types
     */
    static processChanges<T extends Identifiable>(
        oldData: T[], 
        newData: T[],
        filterOptions: FilterGroup
    ): DataChange<T>[] {
        const changes: DataChange<T>[] = [];
        const oldMap = new Map(oldData.map(item => [item.$id, item]));
        const newMap = new Map(newData.map(item => [item.$id, item]));

        // Find deleted and updated items
        for (const [id, oldItem] of oldMap) {
            const newItem = newMap.get(id);
            if (!newItem) {
                // Item exists in old but not in new = deleted
                changes.push({ item: oldItem, type: ChangeType.Deleted });
            } else if (JSON.stringify(oldItem) !== JSON.stringify(newItem)) {
                // Item exists in both but is different = updated
                changes.push({ item: newItem, type: ChangeType.Updated });
            }
        }

        const filterEngine = new FilterEngine();

        // Find added items
        for (const [id, newItem] of newMap) {
            if (!oldMap.has(id)) {
                // Only add if item passes filtering criteria
                const passesFilter = filterOptions ? 
                    filterEngine.process([newItem], filterOptions).length > 0 :
                    true;
                
                if (passesFilter) {
                    changes.push({ item: newItem, type: ChangeType.Added });
                }
            }
        }

        return changes;
    }
}
