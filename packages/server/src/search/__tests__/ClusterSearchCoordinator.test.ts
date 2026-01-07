/**
 * ClusterSearchCoordinator Unit Tests
 *
 * Tests for distributed search across cluster nodes.
 */

import { EventEmitter } from 'events';
import { ClusterSearchCoordinator } from '../ClusterSearchCoordinator';
import { SearchCoordinator } from '../SearchCoordinator';

// Mock ClusterManager
class MockClusterManager extends EventEmitter {
  public readonly config = { nodeId: 'node-1' };
  private sentMessages: Array<{ nodeId: string; type: string; payload: any }> = [];
  private members: string[] = ['node-1'];

  send(nodeId: string, type: string, payload: any) {
    this.sentMessages.push({ nodeId, type, payload });
  }

  getMembers(): string[] {
    return this.members;
  }

  setMembers(members: string[]) {
    this.members = members;
  }

  getSentMessages() {
    return this.sentMessages;
  }

  clearSentMessages() {
    this.sentMessages = [];
  }

  // Simulate receiving a cluster message
  receiveMessage(msg: { type: string; senderId: string; payload: any }) {
    this.emit('message', msg);
  }
}

// Mock PartitionService
class MockPartitionService {
  getPartitionMap() {
    return {
      version: 1,
      partitionCount: 271,
      nodes: [],
      partitions: [],
      generatedAt: Date.now(),
    };
  }
}

// Mock SearchCoordinator
class MockSearchCoordinator extends SearchCoordinator {
  private mockResults: Record<string, any[]> = {};

  setMockResults(mapName: string, results: any[]) {
    this.mockResults[mapName] = results;
  }

  search(mapName: string, _query: string, options?: any): any {
    const allResults = this.mockResults[mapName] || [];
    const limit = options?.limit ?? allResults.length;
    // Return limited results but report total count
    return {
      requestId: '',
      results: allResults.slice(0, limit),
      totalCount: allResults.length,
    };
  }

  isSearchEnabled(_mapName: string): boolean {
    return true;
  }
}

describe('ClusterSearchCoordinator', () => {
  let clusterManager: MockClusterManager;
  let partitionService: MockPartitionService;
  let searchCoordinator: MockSearchCoordinator;
  let clusterSearchCoordinator: ClusterSearchCoordinator;

  beforeEach(() => {
    clusterManager = new MockClusterManager();
    partitionService = new MockPartitionService();
    searchCoordinator = new MockSearchCoordinator();

    clusterSearchCoordinator = new ClusterSearchCoordinator(
      clusterManager as any,
      partitionService as any,
      searchCoordinator,
      { defaultTimeoutMs: 1000 }
    );
  });

  afterEach(() => {
    clusterSearchCoordinator.destroy();
  });

  describe('single-node optimization', () => {
    it('should execute local search when only one node in cluster', async () => {
      clusterManager.setMembers(['node-1']);

      searchCoordinator.setMockResults('articles', [
        { key: 'doc-1', value: { title: 'Test' }, score: 0.95, matchedTerms: ['test'] },
        { key: 'doc-2', value: { title: 'Test 2' }, score: 0.85, matchedTerms: ['test'] },
      ]);

      const result = await clusterSearchCoordinator.search('articles', 'test', { limit: 10 });

      expect(result.results.length).toBe(2);
      expect(result.respondedNodes).toEqual(['node-1']);
      expect(result.failedNodes).toEqual([]);
      expect(result.totalHits).toBe(2);
      // No cluster messages should be sent for single-node
      expect(clusterManager.getSentMessages().length).toBe(0);
    });
  });

  describe('scatter-gather', () => {
    it('should broadcast search to all nodes in cluster', async () => {
      clusterManager.setMembers(['node-1', 'node-2', 'node-3']);

      searchCoordinator.setMockResults('articles', [
        { key: 'doc-1', value: { title: 'Local' }, score: 0.95, matchedTerms: ['test'] },
      ]);

      // Start search (don't await yet)
      const searchPromise = clusterSearchCoordinator.search('articles', 'test', { limit: 10 });

      // Simulate responses from remote nodes
      clusterManager.receiveMessage({
        type: 'CLUSTER_SEARCH_RESP',
        senderId: 'node-2',
        payload: {
          requestId: clusterManager.getSentMessages()[0]?.payload.requestId,
          nodeId: 'node-2',
          results: [
            { key: 'doc-2', value: { title: 'Node 2' }, score: 0.90, matchedTerms: ['test'] },
          ],
          totalHits: 1,
          executionTimeMs: 5,
        },
      });

      clusterManager.receiveMessage({
        type: 'CLUSTER_SEARCH_RESP',
        senderId: 'node-3',
        payload: {
          requestId: clusterManager.getSentMessages()[0]?.payload.requestId,
          nodeId: 'node-3',
          results: [
            { key: 'doc-3', value: { title: 'Node 3' }, score: 0.85, matchedTerms: ['test'] },
          ],
          totalHits: 1,
          executionTimeMs: 3,
        },
      });

      const result = await searchPromise;

      expect(result.results.length).toBe(3);
      expect(result.respondedNodes).toContain('node-1');
      expect(result.respondedNodes).toContain('node-2');
      expect(result.respondedNodes).toContain('node-3');
      expect(result.totalHits).toBe(3);
    });

    it('should merge results using RRF', async () => {
      clusterManager.setMembers(['node-1', 'node-2']);

      // Same document appears on both nodes with different scores
      searchCoordinator.setMockResults('articles', [
        { key: 'doc-common', value: { title: 'Common' }, score: 0.95, matchedTerms: ['test'] },
        { key: 'doc-local', value: { title: 'Local' }, score: 0.70, matchedTerms: ['test'] },
      ]);

      const searchPromise = clusterSearchCoordinator.search('articles', 'test', { limit: 10 });

      // Node-2 also has doc-common (should boost its rank)
      const requestId = clusterManager.getSentMessages()[0]?.payload.requestId;
      clusterManager.receiveMessage({
        type: 'CLUSTER_SEARCH_RESP',
        senderId: 'node-2',
        payload: {
          requestId,
          nodeId: 'node-2',
          results: [
            { key: 'doc-common', value: { title: 'Common' }, score: 0.92, matchedTerms: ['test'] },
            { key: 'doc-remote', value: { title: 'Remote' }, score: 0.80, matchedTerms: ['test'] },
          ],
          totalHits: 2,
          executionTimeMs: 5,
        },
      });

      const result = await searchPromise;

      // doc-common should rank highest (appears in both nodes)
      expect(result.results[0].key).toBe('doc-common');
    });
  });

  describe('timeout handling', () => {
    it('should return partial results on timeout', async () => {
      clusterManager.setMembers(['node-1', 'node-2', 'node-3']);

      searchCoordinator.setMockResults('articles', [
        { key: 'doc-1', value: { title: 'Local' }, score: 0.95, matchedTerms: ['test'] },
      ]);

      // Use short timeout
      const searchPromise = clusterSearchCoordinator.search('articles', 'test', {
        limit: 10,
        timeoutMs: 50,
      });

      // Only node-2 responds before timeout
      const requestId = clusterManager.getSentMessages()[0]?.payload.requestId;
      clusterManager.receiveMessage({
        type: 'CLUSTER_SEARCH_RESP',
        senderId: 'node-2',
        payload: {
          requestId,
          nodeId: 'node-2',
          results: [
            { key: 'doc-2', value: { title: 'Node 2' }, score: 0.90, matchedTerms: ['test'] },
          ],
          totalHits: 1,
          executionTimeMs: 5,
        },
      });

      // Node-3 doesn't respond

      const result = await searchPromise;

      expect(result.results.length).toBe(2); // node-1 + node-2
      expect(result.respondedNodes).toContain('node-1');
      expect(result.respondedNodes).toContain('node-2');
      expect(result.failedNodes).toContain('node-3');
    });
  });

  describe('error handling', () => {
    it('should handle node errors gracefully', async () => {
      clusterManager.setMembers(['node-1', 'node-2', 'node-3']);

      searchCoordinator.setMockResults('articles', [
        { key: 'doc-1', value: { title: 'Local' }, score: 0.95, matchedTerms: ['test'] },
      ]);

      const searchPromise = clusterSearchCoordinator.search('articles', 'test', { limit: 10 });

      const requestId = clusterManager.getSentMessages()[0]?.payload.requestId;

      // Node-2 returns error
      clusterManager.receiveMessage({
        type: 'CLUSTER_SEARCH_RESP',
        senderId: 'node-2',
        payload: {
          requestId,
          nodeId: 'node-2',
          results: [],
          totalHits: 0,
          executionTimeMs: 5,
          error: 'Index not available',
        },
      });

      // Node-3 returns success
      clusterManager.receiveMessage({
        type: 'CLUSTER_SEARCH_RESP',
        senderId: 'node-3',
        payload: {
          requestId,
          nodeId: 'node-3',
          results: [
            { key: 'doc-3', value: { title: 'Node 3' }, score: 0.85, matchedTerms: ['test'] },
          ],
          totalHits: 1,
          executionTimeMs: 3,
        },
      });

      const result = await searchPromise;

      expect(result.results.length).toBe(2); // node-1 + node-3
      expect(result.respondedNodes).toEqual(['node-1', 'node-3']);
      expect(result.failedNodes).toEqual(['node-2']);
    });
  });

  describe('cursor pagination', () => {
    it('should generate nextCursor when more results available', async () => {
      clusterManager.setMembers(['node-1']);

      // More results than limit
      const manyResults = Array.from({ length: 20 }, (_, i) => ({
        key: `doc-${i}`,
        value: { title: `Doc ${i}` },
        score: 1 - i * 0.01,
        matchedTerms: ['test'],
      }));
      searchCoordinator.setMockResults('articles', manyResults);

      const result = await clusterSearchCoordinator.search('articles', 'test', { limit: 5 });

      expect(result.results.length).toBe(5);
      expect(result.nextCursor).toBeDefined();
    });

    it('should not generate nextCursor when all results returned', async () => {
      clusterManager.setMembers(['node-1']);

      searchCoordinator.setMockResults('articles', [
        { key: 'doc-1', value: { title: 'Doc 1' }, score: 0.95, matchedTerms: ['test'] },
        { key: 'doc-2', value: { title: 'Doc 2' }, score: 0.85, matchedTerms: ['test'] },
      ]);

      const result = await clusterSearchCoordinator.search('articles', 'test', { limit: 10 });

      expect(result.results.length).toBe(2);
      expect(result.nextCursor).toBeUndefined();
    });
  });

  describe('configuration', () => {
    it('should use default RRF k value', () => {
      expect(clusterSearchCoordinator.getRrfK()).toBe(60);
    });

    it('should use custom RRF k value', () => {
      const coordinator = new ClusterSearchCoordinator(
        clusterManager as any,
        partitionService as any,
        searchCoordinator,
        { rrfK: 45 }
      );

      expect(coordinator.getRrfK()).toBe(45);
      coordinator.destroy();
    });
  });

  describe('destroy', () => {
    it('should reject pending requests on destroy', async () => {
      clusterManager.setMembers(['node-1', 'node-2']);

      searchCoordinator.setMockResults('articles', []);

      const searchPromise = clusterSearchCoordinator.search('articles', 'test', { limit: 10 });

      // Destroy before responses arrive
      clusterSearchCoordinator.destroy();

      await expect(searchPromise).rejects.toThrow('ClusterSearchCoordinator destroyed');
    });
  });
});
