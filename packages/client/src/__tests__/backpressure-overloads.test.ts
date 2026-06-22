import { BackpressureController } from '../sync/BackpressureController';
import type { BackpressureConfig } from '../BackpressureConfig';
import type { OpLogEntry } from '../SyncEngine';
import type { BackpressureThresholdEvent, OperationDroppedEvent } from '../BackpressureConfig';

function makeOp(id: number): OpLogEntry {
  return {
    id: String(id),
    mapName: 'users',
    opType: 'PUT',
    key: `k${id}`,
    synced: false,
  } as unknown as OpLogEntry;
}

function makeController(
  opLog: OpLogEntry[],
  overrides: Partial<BackpressureConfig> = {},
): BackpressureController {
  const config: BackpressureConfig = {
    maxPendingOps: 10,
    strategy: 'pause',
    highWaterMark: 0.8,
    lowWaterMark: 0.5,
    ...overrides,
  };
  return new BackpressureController({ config, opLog });
}

describe('onBackpressure typed overloads', () => {
  // Each test subscribes with a listener whose parameter is destructured/typed
  // per the documented overload. If the overloads did not narrow, these would
  // fail to compile (TS2339/TS2459) — so this suite is both a runtime and a
  // type-level check.

  it("'backpressure:high' narrows to BackpressureThresholdEvent", () => {
    const opLog: OpLogEntry[] = [];
    const controller = makeController(opLog);
    let received: BackpressureThresholdEvent | undefined;

    controller.onBackpressure('backpressure:high', ({ pending, max }) => {
      received = { pending, max };
    });

    for (let i = 0; i < 8; i++) opLog.push(makeOp(i)); // threshold = floor(10 * 0.8) = 8
    controller.checkHighWaterMark();

    expect(received).toEqual({ pending: 8, max: 10 });
  });

  it("'backpressure:low' narrows to BackpressureThresholdEvent and resumes", () => {
    const opLog: OpLogEntry[] = [];
    const controller = makeController(opLog);
    let low: BackpressureThresholdEvent | undefined;
    let resumed = false;

    controller.onBackpressure('backpressure:low', ({ pending, max }) => {
      low = { pending, max };
    });
    controller.onBackpressure('backpressure:resumed', () => {
      resumed = true;
    });

    // Saturate to trip the pause strategy, then drain below the low water mark.
    for (let i = 0; i < 10; i++) opLog.push(makeOp(i));
    void controller.checkBackpressure(); // strategy 'pause' → backpressurePaused = true
    opLog.length = 4; // below floor(10 * 0.5) = 5
    controller.checkLowWaterMark();

    expect(low).toEqual({ pending: 4, max: 10 });
    expect(resumed).toBe(true);
  });

  it("'backpressure:paused' takes a payload-free listener", async () => {
    const opLog: OpLogEntry[] = [];
    const controller = makeController(opLog);
    let paused = false;

    controller.onBackpressure('backpressure:paused', () => {
      paused = true;
    });

    for (let i = 0; i < 10; i++) opLog.push(makeOp(i));
    // 'pause' strategy blocks until capacity; don't await (it resolves on drain).
    void controller.checkBackpressure();

    expect(paused).toBe(true);
  });

  it("'operation:dropped' narrows to OperationDroppedEvent", async () => {
    const opLog: OpLogEntry[] = [];
    const controller = makeController(opLog, { maxPendingOps: 2, strategy: 'drop-oldest' });
    let dropped: OperationDroppedEvent | undefined;

    controller.onBackpressure('operation:dropped', ({ opId, mapName, opType, key }) => {
      dropped = { opId, mapName, opType, key };
    });

    opLog.push(makeOp(0));
    opLog.push(makeOp(1));
    await controller.checkBackpressure(); // at capacity → drop oldest (id "0")

    expect(dropped).toEqual({ opId: '0', mapName: 'users', opType: 'PUT', key: 'k0' });
  });
});
