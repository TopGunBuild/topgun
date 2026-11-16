/**
 * SubscriptionRegistry.start: how the tool's `filter` becomes the live query.
 */

import type { TopGunClient } from '@topgunbuild/client';
import { SubscriptionRegistry } from '../subscriptions';
import { SubscribeArgsSchema, toolSchemas } from '../schemas';

function stubClient(recorded: unknown[]): TopGunClient {
  const handle = {
    onDelta: () => () => {},
    subscribe: () => () => {},
    whenSettled: () => Promise.resolve(),
    isSettled: true,
  };
  return {
    query: (_map: string, filter: unknown) => {
      recorded.push(filter);
      return handle;
    },
  } as unknown as TopGunClient;
}

describe('SubscriptionRegistry.start filter mapping', () => {
  let recorded: unknown[];
  let registry: SubscriptionRegistry;

  beforeEach(() => {
    recorded = [];
    registry = new SubscriptionRegistry(stubClient(recorded));
  });

  afterEach(() => {
    registry.teardownAll();
  });

  it('sends a field filter as where and lists the filter the agent sent', async () => {
    const result = await registry.start('tasks', { status: 'open' }, 60_000);

    expect(result.ok).toBe(true);
    expect(recorded).toEqual([{ where: { status: 'open' } }]);
    expect(registry.list()[0].filter).toEqual({ status: 'open' });
  });

  it('sends an empty filter as a query with no where', async () => {
    const result = await registry.start('tasks', {}, 60_000);

    expect(result.ok).toBe(true);
    expect(recorded).toEqual([{}]);
    expect(recorded[0]).not.toHaveProperty('where');
  });

  it('treats QueryFilter option names in the filter as field names', async () => {
    const filter = { where: { a: 1 }, limit: 5, sort: { a: 'asc' }, groupBy: ['x'] };
    const result = await registry.start('tasks', filter, 60_000);

    expect(result.ok).toBe(true);
    expect(recorded).toEqual([{ where: filter }]);
    expect(Object.keys(recorded[0] as object)).toEqual(['where']);
  });
});

describe('filter description in the tool schemas', () => {
  it('says the query and subscribe filters are field equality', () => {
    const subscribeText = toolSchemas.subscribe.properties.filter.description;
    const queryText = toolSchemas.query.properties.filter.description;

    expect(subscribeText).toContain('equal');
    expect(queryText).toContain('equal');
    expect(subscribeText).toBe(SubscribeArgsSchema.shape.filter.description);
  });
});
