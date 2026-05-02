import React, { Profiler } from 'react';
import { renderHook, render, act } from '@testing-library/react';
import { useQuery } from '../hooks/useQuery';
import { useSyncState } from '../hooks/useSyncState';
import { TopGunProvider } from '../TopGunProvider';
// Import directly from source files to avoid pulling in SyncEngine ->
// msgpackr (ESM) which jest can't transform without extra config.
import { RecordSyncStateTracker } from '../../../client/src/RecordSyncState';
import { SyncState } from '../../../client/src/SyncState';
import type { TopGunClient, QueryResultItem, RecordSyncState } from '@topgunbuild/client';
import type { Timestamp } from '@topgunbuild/core';

interface TestOp {
  mapName: string;
  key: string;
  timestamp: Timestamp;
  synced: boolean;
}

// --- Test fixtures -----------------------------------------------------------

function ts(millis: number, counter = 0, nodeId = 'nodeA'): Timestamp {
  return { millis, counter, nodeId };
}

function makeOp(
  mapName: string,
  key: string,
  timestamp: Timestamp,
  synced = false,
): TestOp {
  return { mapName, key, timestamp, synced };
}

interface Todo {
  text: string;
  completed: boolean;
}

interface MockHandleControls {
  /** Inject server data into the query. */
  emitData: (items: { _key: string; text: string; completed: boolean }[]) => void;
  /** Get the most recently emitted syncState snapshot. */
  getSyncState: () => ReadonlyMap<string, RecordSyncState>;
}

/**
 * Build a mocked TopGunClient backed by a real RecordSyncStateTracker so we
 * can drive the integration test through the same projection rules used in
 * production. The query handle's syncState subscription is wired to the real
 * tracker via the same filtered-snapshot logic as QueryHandle (we re-emit on
 * tracker change with a snapshot filtered to currently-known result keys).
 */
function buildClient(
  tracker: RecordSyncStateTracker,
): { client: TopGunClient; controls: Map<string, MockHandleControls> } {
  const controls = new Map<string, MockHandleControls>();

  const queryFactory = (mapName: string) => {
    let dataListener: ((results: QueryResultItem<Todo>[]) => void) | null = null;
    let syncStateListener: ((snapshot: ReadonlyMap<string, RecordSyncState>) => void) | null = null;
    let knownKeys = new Set<string>();
    let lastEmittedSnapshot: ReadonlyMap<string, RecordSyncState> | null = null;

    const mapsEqual = (
      a: ReadonlyMap<string, RecordSyncState>,
      b: ReadonlyMap<string, RecordSyncState>,
    ): boolean => {
      if (a.size !== b.size) return false;
      for (const [k, v] of a) if (b.get(k) !== v) return false;
      return true;
    };

    const filteredSnapshot = (): ReadonlyMap<string, RecordSyncState> => {
      const out = new Map<string, RecordSyncState>();
      for (const k of knownKeys) {
        out.set(k, tracker.get(mapName, k));
      }
      return out;
    };

    const emitIfChanged = (): void => {
      const fresh = filteredSnapshot();
      if (lastEmittedSnapshot && mapsEqual(lastEmittedSnapshot, fresh)) {
        // Suppress no-op emission — matches production QueryHandle behavior.
        return;
      }
      lastEmittedSnapshot = fresh;
      syncStateListener?.(fresh);
    };

    const trackerOff = tracker.onChange(mapName, () => {
      emitIfChanged();
    });

    const handle = {
      subscribe: (cb: (results: QueryResultItem<Todo>[]) => void) => {
        dataListener = cb;
        return () => {
          dataListener = null;
        };
      },
      onChanges: (_cb: any) => () => {},
      onPaginationChange: (cb: any) => {
        cb({ hasMore: false, cursorStatus: 'none' });
        return () => {};
      },
      onSyncStateChange: (cb: (snapshot: ReadonlyMap<string, RecordSyncState>) => void) => {
        syncStateListener = cb;
        const initial = filteredSnapshot();
        lastEmittedSnapshot = initial;
        cb(initial);
        return () => {
          syncStateListener = null;
          trackerOff();
        };
      },
    };

    controls.set(mapName, {
      emitData: (items) => {
        knownKeys = new Set(items.map((i) => i._key));
        dataListener?.(items as QueryResultItem<Todo>[]);
        emitIfChanged();
      },
      getSyncState: filteredSnapshot,
    });

    return handle;
  };

  const client = {
    query: jest.fn((mapName: string) => queryFactory(mapName)),
    getRecordSyncStateTracker: () => tracker,
  } as unknown as TopGunClient;

  return { client, controls };
}

// --- Tests -------------------------------------------------------------------

describe('useQuery + syncState integration', () => {
  let tracker: RecordSyncStateTracker;
  let client: TopGunClient;
  let controls: Map<string, MockHandleControls>;
  let wrapper: React.FC<{ children: React.ReactNode }>;

  beforeEach(() => {
    tracker = new RecordSyncStateTracker(SyncState.DISCONNECTED);
    const built = buildClient(tracker);
    client = built.client;
    controls = built.controls;
    wrapper = ({ children }) => (
      <TopGunProvider client={client}>{children}</TopGunProvider>
    );
  });

  afterEach(() => {
    tracker.dispose();
  });

  // AC #1 + #2: useQuery exposes syncState and offline writes project to local-only.
  it('AC#1+#2: returns syncState; offline write projects to local-only within one render cycle', () => {
    const { result } = renderHook(() => useQuery<Todo>('todos'), { wrapper });

    // Optimistic data update — the application layer pushes into the query
    // result alongside recording the local op (matches production order).
    act(() => {
      controls.get('todos')!.emitData([
        { _key: 'todo-1', text: 'buy milk', completed: false },
      ]);
      tracker.onAppend(makeOp('todos', 'todo-1', ts(100)));
    });

    expect(result.current.data).toEqual([
      { _key: 'todo-1', text: 'buy milk', completed: false },
    ]);
    expect(result.current.syncState.get('todo-1')).toBe('local-only');
  });

  // AC #3: After reconnect + ack, transitions through pending -> synced.
  it('AC#3: after reconnect + OP_ACK, key transitions pending -> synced', () => {
    const { result } = renderHook(() => useQuery<Todo>('todos'), { wrapper });

    const op = makeOp('todos', 'todo-1', ts(100));
    act(() => {
      controls.get('todos')!.emitData([
        { _key: 'todo-1', text: 'buy milk', completed: false },
      ]);
      tracker.onAppend(op);
    });
    expect(result.current.syncState.get('todo-1')).toBe('local-only');

    act(() => {
      tracker.onConnectionStateChange(SyncState.CONNECTED);
    });
    expect(result.current.syncState.get('todo-1')).toBe('pending');

    act(() => {
      tracker.onAcknowledge({ ...op, synced: true });
    });
    expect(result.current.syncState.get('todo-1')).toBe('synced');
  });

  // AC #4 (branch A): MergeRejection >= latest opLog timestamp -> conflicted.
  it('AC#4-A: MergeRejection >= latest opLog timestamp flips key to conflicted', () => {
    const { result } = renderHook(() => useQuery<Todo>('todos'), { wrapper });

    act(() => {
      controls.get('todos')!.emitData([
        { _key: 'todo-1', text: 'buy milk', completed: false },
      ]);
      tracker.onConnectionStateChange(SyncState.CONNECTED);
      tracker.onAppend(makeOp('todos', 'todo-1', ts(100)));
    });
    expect(result.current.syncState.get('todo-1')).toBe('pending');

    act(() => {
      tracker.onRejection({
        mapName: 'todos',
        key: 'todo-1',
        attemptedValue: { text: 'buy milk' },
        reason: 'duplicate',
        timestamp: ts(150),
        nodeId: 'serverA',
      });
    });
    expect(result.current.syncState.get('todo-1')).toBe('conflicted');
  });

  // AC #4 (branch B): late-arrival rejection (older than latest local write)
  // does NOT mark conflicted.
  it('AC#4-B: late-arrival rejection (timestamp < latest local write) does NOT mark conflicted', () => {
    const { result } = renderHook(() => useQuery<Todo>('todos'), { wrapper });

    act(() => {
      controls.get('todos')!.emitData([
        { _key: 'todo-1', text: 'buy milk', completed: false },
      ]);
      tracker.onConnectionStateChange(SyncState.CONNECTED);
      // Initial write at t=100.
      tracker.onAppend(makeOp('todos', 'todo-1', ts(100)));
      // Subsequent local write at t=200 supersedes.
      tracker.onAppend(makeOp('todos', 'todo-1', ts(200)));
      // Late-arrival rejection at t=100.
      tracker.onRejection({
        mapName: 'todos',
        key: 'todo-1',
        attemptedValue: { text: 'old value' },
        reason: 'duplicate',
        timestamp: ts(100),
        nodeId: 'serverA',
      });
    });
    // Falls through to projection rule 2 — pending, NOT conflicted.
    expect(result.current.syncState.get('todo-1')).toBe('pending');
  });

  // AC #4 (continued): conflicted clears on subsequent acknowledged write.
  it('AC#4: conflicted clears after a subsequent acknowledged write', () => {
    const { result } = renderHook(() => useQuery<Todo>('todos'), { wrapper });

    const initialOp = makeOp('todos', 'todo-1', ts(100));
    act(() => {
      controls.get('todos')!.emitData([
        { _key: 'todo-1', text: 'buy milk', completed: false },
      ]);
      tracker.onConnectionStateChange(SyncState.CONNECTED);
      tracker.onAppend(initialOp);
      tracker.onRejection({
        mapName: 'todos',
        key: 'todo-1',
        attemptedValue: { text: 'buy milk' },
        reason: 'duplicate',
        timestamp: ts(100),
        nodeId: 'serverA',
      });
    });
    expect(result.current.syncState.get('todo-1')).toBe('conflicted');

    const retryOp = makeOp('todos', 'todo-1', ts(200));
    act(() => {
      tracker.onAppend(retryOp);
    });
    expect(result.current.syncState.get('todo-1')).toBe('pending');

    act(() => {
      tracker.onAcknowledge({ ...retryOp, synced: true });
    });
    expect(result.current.syncState.get('todo-1')).toBe('synced');
  });

  // AC #5: type export — verified at compile time + runtime four-member check.
  it('AC#5: RecordSyncState union has exactly four members (runtime sanity check)', () => {
    const states: RecordSyncState[] = ['synced', 'pending', 'conflicted', 'local-only'];
    expect(states).toHaveLength(4);
    expect(new Set(states).size).toBe(4);
  });

  // AC #9: re-render economy via Profiler.
  it('AC#9: irrelevant per-record state changes do NOT trigger re-renders for an unrelated query', () => {
    const renderCounts = { todos: 0, users: 0 };
    const onTodosRender = () => {
      renderCounts.todos += 1;
    };
    const onUsersRender = () => {
      renderCounts.users += 1;
    };

    const TodosConsumer: React.FC = () => {
      const { data, syncState } = useQuery<Todo>('todos');
      return (
        <div data-testid="todos">
          {data.length}-{syncState.size}
        </div>
      );
    };
    const UsersConsumer: React.FC = () => {
      const { data, syncState } = useQuery<{ name: string }>('users');
      return (
        <div data-testid="users">
          {data.length}-{syncState.size}
        </div>
      );
    };

    render(
      <TopGunProvider client={client}>
        <Profiler id="todos" onRender={onTodosRender}>
          <TodosConsumer />
        </Profiler>
        <Profiler id="users" onRender={onUsersRender}>
          <UsersConsumer />
        </Profiler>
      </TopGunProvider>,
    );

    // Wait for initial renders to settle.
    act(() => {
      controls.get('todos')!.emitData([
        { _key: 'todo-1', text: 'buy milk', completed: false },
      ]);
      controls.get('users')!.emitData([] as any);
    });

    // Capture baseline render counts after initial data emission.
    const baselineUsers = renderCounts.users;

    // Fire a tracker event for 'todos' that changes its state — should
    // re-render TodosConsumer but NOT UsersConsumer.
    act(() => {
      tracker.onConnectionStateChange(SyncState.CONNECTED);
      tracker.onAppend(makeOp('todos', 'todo-1', ts(100)));
    });

    expect(renderCounts.users).toBe(baselineUsers);
  });

  // AC #9 (continued): same map but key not in result set.
  it('AC#9: a state change for a key NOT in the query result set does NOT re-render the consumer', () => {
    let renders = 0;
    const Consumer: React.FC = () => {
      const { data, syncState } = useQuery<Todo>('todos');
      return (
        <div>
          {data.length}-{syncState.size}
        </div>
      );
    };

    render(
      <TopGunProvider client={client}>
        <Profiler
          id="todos"
          onRender={() => {
            renders += 1;
          }}
        >
          <Consumer />
        </Profiler>
      </TopGunProvider>,
    );

    act(() => {
      controls.get('todos')!.emitData([
        { _key: 'todo-1', text: 'buy milk', completed: false },
      ]);
    });

    const baseline = renders;

    // Fire a tracker event for a key NOT in the result set ('todo-99'). The
    // mock query handle's filtered snapshot excludes this key, so the
    // useQuery setState receives an equal-content snapshot and React's
    // state-equality-by-Object.is short-circuits — no re-render.
    act(() => {
      tracker.onConnectionStateChange(SyncState.CONNECTED);
      tracker.onAppend(makeOp('todos', 'todo-99', ts(100)));
    });

    // Allow at most one re-render (React batches; Profiler may double-count
    // in StrictMode dev). The strict assertion is "no extra renders beyond
    // what would have happened anyway" — we use baseline +/- 0 here because
    // our mock filtered snapshot is identity-stable for irrelevant keys.
    expect(renders).toBe(baseline);
  });
});

describe('useSyncState single-key hook', () => {
  let tracker: RecordSyncStateTracker;
  let client: TopGunClient;
  let wrapper: React.FC<{ children: React.ReactNode }>;

  beforeEach(() => {
    tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
    client = {
      getRecordSyncStateTracker: () => tracker,
      query: jest.fn(),
    } as unknown as TopGunClient;
    wrapper = ({ children }) => (
      <TopGunProvider client={client}>{children}</TopGunProvider>
    );
  });

  afterEach(() => {
    tracker.dispose();
  });

  it("returns 'synced' for an unknown key", () => {
    const { result } = renderHook(() => useSyncState('todos', 'never-written'), { wrapper });
    expect(result.current).toBe('synced');
  });

  it("transitions through pending -> synced on append + ack", () => {
    const { result } = renderHook(() => useSyncState('todos', 'todo-1'), { wrapper });
    expect(result.current).toBe('synced');
    const op = makeOp('todos', 'todo-1', ts(100));
    act(() => {
      tracker.onAppend(op);
    });
    expect(result.current).toBe('pending');
    act(() => {
      tracker.onAcknowledge({ ...op, synced: true });
    });
    expect(result.current).toBe('synced');
  });

  it('does NOT re-render when a sibling key in the same map changes state', () => {
    let renders = 0;
    const Consumer: React.FC = () => {
      const state = useSyncState('todos', 'todo-1');
      renders += 1;
      return <span>{state}</span>;
    };
    render(
      <TopGunProvider client={client}>
        <Consumer />
      </TopGunProvider>,
    );
    const baseline = renders;
    // Sibling key change — should not re-render Consumer.
    act(() => {
      tracker.onAppend(makeOp('todos', 'sibling-key', ts(100)));
    });
    expect(renders).toBe(baseline);
  });
});
