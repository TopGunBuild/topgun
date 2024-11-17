/** Custom error types for better error handling */
export class StoreError extends Error {
    constructor(message: string, public readonly code: string) {
        super(message);
        this.name = 'StoreError';
    }
}
