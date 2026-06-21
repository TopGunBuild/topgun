import { createRustTestContext, createLWWRecord, waitForSync, RustTestContext } from './helpers';

/**
 * Event Journal end-to-end (TS wire protocol → Rust server).
 *
 * Proves the write-path wiring that getEventJournal / useEventJournal depend on:
 * a CLIENT_OP applied on the server is appended to the journal and pushed to
 * matching JOURNAL_SUBSCRIBE connections as a single-nested JOURNAL_EVENT, and
 * is retrievable via JOURNAL_READ. Before this wiring the server never called
 * append(), so subscribers received nothing and reads were always empty — the
 * silent dark-feature this closes (TODO-459; the journal slice of TODO-464).
 */
describe('Integration: Event Journal (Rust Server)', () => {
  let ctx: RustTestContext;

  beforeAll(async () => {
    ctx = await createRustTestContext(2);
  });

  afterAll(async () => {
    await ctx.cleanup();
  });

  test('JOURNAL_SUBSCRIBE receives a JOURNAL_EVENT push after a server write', async () => {
    const [writer, watcher] = ctx.clients;

    watcher.messages.length = 0;
    watcher.send({
      type: 'JOURNAL_SUBSCRIBE',
      requestId: 'sub-1',
      mapName: 'todos',
    });
    // JOURNAL_SUBSCRIBE has no ack; give the persistence service time to register
    // the subscription before the write triggers the push.
    await waitForSync(200);

    writer.send({
      type: 'CLIENT_OP',
      payload: {
        id: 'jop-1',
        mapName: 'todos',
        opType: 'PUT',
        key: 'todo-1',
        record: createLWWRecord({ title: 'Write the journal test', done: false }),
      },
    });

    const evt = await watcher.waitForMessage('JOURNAL_EVENT');
    expect(evt).toBeDefined();
    // Single-nested wire shape: `event` IS the JournalEventData (not event.event).
    expect(evt.event).toBeDefined();
    expect(evt.event.event).toBeUndefined();
    expect(evt.event.type).toBe('PUT');
    expect(evt.event.mapName).toBe('todos');
    expect(evt.event.key).toBe('todo-1');
    // Sequence is a monotonic string assigned by the store.
    expect(typeof evt.event.sequence).toBe('string');
    expect(Number(evt.event.sequence)).toBeGreaterThan(0);
  });

  test('JOURNAL_READ returns appended events', async () => {
    const [writer] = ctx.clients;

    writer.send({
      type: 'CLIENT_OP',
      payload: {
        id: 'jop-read-1',
        mapName: 'orders',
        opType: 'PUT',
        key: 'order-1',
        record: createLWWRecord({ total: 42 }),
      },
    });
    await writer.waitForMessage('OP_ACK');
    await waitForSync(150);

    writer.messages.length = 0;
    writer.send({
      type: 'JOURNAL_READ',
      requestId: 'read-1',
      fromSequence: '0',
      limit: 100,
      mapName: 'orders',
    });

    const resp = await writer.waitForMessage('JOURNAL_READ_RESPONSE');
    expect(resp.requestId).toBe('read-1');
    expect(Array.isArray(resp.events)).toBe(true);
    const orderEvents = resp.events.filter((e: { mapName: string }) => e.mapName === 'orders');
    expect(orderEvents.length).toBeGreaterThanOrEqual(1);
    expect(orderEvents.some((e: { key: string }) => e.key === 'order-1')).toBe(true);
  });

  test('a REMOVE is journaled as a DELETE event', async () => {
    const [writer, watcher] = ctx.clients;

    watcher.messages.length = 0;
    watcher.send({
      type: 'JOURNAL_SUBSCRIBE',
      requestId: 'sub-del',
      mapName: 'inbox',
      types: ['DELETE'],
    });
    await waitForSync(200);

    // A PUT then a REMOVE on the same key; the type filter must drop the PUT and
    // deliver only the DELETE.
    writer.send({
      type: 'CLIENT_OP',
      payload: {
        id: 'jop-del-put',
        mapName: 'inbox',
        opType: 'PUT',
        key: 'msg-1',
        record: createLWWRecord({ body: 'hi' }),
      },
    });
    await writer.waitForMessage('OP_ACK');

    writer.send({
      type: 'CLIENT_OP',
      payload: {
        id: 'jop-del-rm',
        mapName: 'inbox',
        opType: 'REMOVE',
        key: 'msg-1',
        record: null,
      },
    });

    const evt = await watcher.waitForMessage('JOURNAL_EVENT');
    expect(evt.event.type).toBe('DELETE');
    expect(evt.event.mapName).toBe('inbox');
    expect(evt.event.key).toBe('msg-1');
  });
});
