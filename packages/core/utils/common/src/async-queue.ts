export class AsyncQueue
{
    private queue: (() => Promise<any>)[] = [];
    private isProcessing: boolean         = false;

    /**
     * Method to add a task to the queue
     *
     * @param {() => Promise<any>} task
     */
    enqueue(task: () => Promise<any>): void
    {
        this.queue.push(task);
        this.processQueue();
    }

    /**
     * Method to destroy the queue
     */
    destroy(): void
    {
        this.queue = [];
    }

    /**
     * Method to process the queue
     *
     * @returns {Promise<void>}
     * @private
     */
    private async processQueue(): Promise<void>
    {
        if (this.isProcessing) return; // Prevent concurrent processing
        this.isProcessing = true;

        while (this.queue.length > 0)
        {
            const currentTask = this.queue.shift(); // Get the next task
            if (currentTask)
            {
                try
                {
                    await currentTask(); // Execute the task
                }
                catch (error)
                {
                    console.error('Task failed:', error);
                }
            }
        }

        this.isProcessing = false; // Reset processing state
    }
}
