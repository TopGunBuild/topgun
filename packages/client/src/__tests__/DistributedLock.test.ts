import { DistributedLock } from '../DistributedLock';
import { SyncEngine } from '../SyncEngine';

// Mock SyncEngine
const mockSyncEngine = {
  requestLock: jest.fn(),
  releaseLock: jest.fn(),
} as unknown as SyncEngine;

describe('DistributedLock', () => {
  let lock: DistributedLock;

  beforeEach(() => {
    jest.clearAllMocks();
    lock = new DistributedLock(mockSyncEngine, 'test-lock');
  });

  test('should acquire lock successfully', async () => {
    (mockSyncEngine.requestLock as jest.Mock).mockResolvedValue({ fencingToken: 10 });

    const result = await lock.lock(5000);

    expect(result).toBe(true);
    expect(lock.isLocked()).toBe(true);
    expect(mockSyncEngine.requestLock).toHaveBeenCalledWith('test-lock', expect.any(String), 5000);
  });

  test('should fail to acquire lock if request fails', async () => {
    (mockSyncEngine.requestLock as jest.Mock).mockRejectedValue(new Error('Timeout'));

    const result = await lock.lock();

    expect(result).toBe(false);
    expect(lock.isLocked()).toBe(false);
  });

  test('should unlock successfully', async () => {
    // Setup locked state
    (mockSyncEngine.requestLock as jest.Mock).mockResolvedValue({ fencingToken: 10 });
    await lock.lock();

    await lock.unlock();

    expect(lock.isLocked()).toBe(false);
    expect(mockSyncEngine.releaseLock).toHaveBeenCalledWith('test-lock', expect.any(String), 10);
  });

  test('should do nothing on unlock if not locked', async () => {
    await lock.unlock();
    expect(mockSyncEngine.releaseLock).not.toHaveBeenCalled();
  });
});

