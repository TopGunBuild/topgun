import { Lockbox } from '@topgunbuild/models';

/**
 * Returns the lockbox with the highest generation from an array of lockboxes.
 * Returns null if the array is empty.
 * 
 * @param lockboxes Array of lockboxes to search through
 * @returns The lockbox with the highest generation or null
 */
export function getLockboxLatestGeneration(lockboxes: Lockbox[]): Lockbox | null {
    if (!lockboxes?.length) {
        return null;
    }

    return lockboxes.reduce((highest, current) => 
        !highest || current.generation > highest.generation ? current : highest
    , lockboxes[0]);
}
