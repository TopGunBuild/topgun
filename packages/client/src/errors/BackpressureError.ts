/**
 * Error thrown when backpressure limit is reached and strategy is 'throw'.
 */
export class BackpressureError extends Error {
  public readonly name = 'BackpressureError';

  constructor(
    public readonly pendingCount: number,
    public readonly maxPending: number
  ) {
    super(
      `Backpressure limit reached: ${pendingCount}/${maxPending} pending operations. ` +
      `Wait for acknowledgments or increase maxPendingOps.`
    );

    // Maintains proper stack trace for where error was thrown (only available on V8)
    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, BackpressureError);
    }
  }
}
