import {
  HybridSearchMessageSchema,
  HybridSearchRespMessageSchema,
  HybridSearchRespPayloadSchema,
  HybridSearchSubMessageSchema,
  HybridSearchUpdateMessageSchema,
  HybridSearchUnsubMessageSchema,
} from '../hybrid-search-schemas';

describe('HybridSearchSchemas', () => {
  describe('HybridSearchMessageSchema', () => {
    it('parses a well-formed HYBRID_SEARCH message', () => {
      const input = {
        type: 'HYBRID_SEARCH',
        payload: {
          requestId: 'req-1',
          mapName: 'products',
          queryText: 'wireless headphones',
          methods: ['fullText', 'semantic'],
          k: 10,
        },
      };
      const result = HybridSearchMessageSchema.safeParse(input);
      expect(result.success).toBe(true);
    });

    it('parses with all optional fields', () => {
      const input = {
        type: 'HYBRID_SEARCH',
        payload: {
          requestId: 'req-2',
          mapName: 'products',
          queryText: 'noise cancelling',
          methods: ['exact', 'fullText', 'semantic'],
          k: 5,
          queryVector: new Uint8Array([0, 0, 128, 63]),
          predicate: { field: 'category', op: 'eq', value: 'audio' },
          includeValue: false,
          minScore: 0.5,
        },
      };
      const result = HybridSearchMessageSchema.safeParse(input);
      expect(result.success).toBe(true);
    });

    it('rejects missing required fields', () => {
      const input = {
        type: 'HYBRID_SEARCH',
        payload: { requestId: 'req-3' },
      };
      const result = HybridSearchMessageSchema.safeParse(input);
      expect(result.success).toBe(false);
    });

    it('rejects wrong type literal', () => {
      const input = {
        type: 'SEARCH',
        payload: {
          requestId: 'req-4',
          mapName: 'products',
          queryText: 'test',
          methods: ['exact'],
          k: 10,
        },
      };
      const result = HybridSearchMessageSchema.safeParse(input);
      expect(result.success).toBe(false);
    });
  });

  describe('HybridSearchRespPayloadSchema', () => {
    it('parses a well-formed response payload', () => {
      const input = {
        requestId: 'req-1',
        results: [
          {
            key: 'prod-42',
            score: 0.85,
            methodScores: { fullText: 0.9, semantic: 0.8 },
          },
        ],
        searchTimeMs: 12,
      };
      const result = HybridSearchRespPayloadSchema.safeParse(input);
      expect(result.success).toBe(true);
    });

    it('parses an error response', () => {
      const input = {
        requestId: 'req-1',
        results: [],
        searchTimeMs: 0,
        error: 'index registry not found for map',
      };
      const result = HybridSearchRespPayloadSchema.safeParse(input);
      expect(result.success).toBe(true);
    });

    it('parses results with values', () => {
      const input = {
        requestId: 'req-1',
        results: [
          {
            key: 'prod-1',
            score: 0.95,
            methodScores: { exact: 1.0 },
            value: { name: 'Headphones', price: 99 },
          },
        ],
        searchTimeMs: 5,
      };
      const result = HybridSearchRespPayloadSchema.safeParse(input);
      expect(result.success).toBe(true);
    });
  });

  describe('HybridSearchRespMessageSchema', () => {
    it('parses a well-formed HYBRID_SEARCH_RESP message', () => {
      const input = {
        type: 'HYBRID_SEARCH_RESP',
        payload: {
          requestId: 'req-1',
          results: [],
          searchTimeMs: 0,
        },
      };
      const result = HybridSearchRespMessageSchema.safeParse(input);
      expect(result.success).toBe(true);
    });
  });

  describe('HybridSearchSubMessageSchema', () => {
    it('parses a well-formed HYBRID_SEARCH_SUB message', () => {
      const input = {
        type: 'HYBRID_SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-1',
          mapName: 'products',
          queryText: 'headphones',
          methods: ['fullText'],
          k: 10,
        },
      };
      const result = HybridSearchSubMessageSchema.safeParse(input);
      expect(result.success).toBe(true);
    });
  });

  describe('HybridSearchUpdateMessageSchema', () => {
    it('parses a well-formed HYBRID_SEARCH_UPDATE message', () => {
      const input = {
        type: 'HYBRID_SEARCH_UPDATE',
        payload: {
          subscriptionId: 'sub-1',
          key: 'prod-42',
          score: 0.9,
          methodScores: { fullText: 0.9 },
          changeType: 'ENTER',
        },
      };
      const result = HybridSearchUpdateMessageSchema.safeParse(input);
      expect(result.success).toBe(true);
    });

    it('parses an UPDATE change type with value', () => {
      const input = {
        type: 'HYBRID_SEARCH_UPDATE',
        payload: {
          subscriptionId: 'sub-1',
          key: 'prod-42',
          score: 0.85,
          methodScores: { fullText: 0.85, semantic: 0.8 },
          value: { name: 'Updated Product' },
          changeType: 'UPDATE',
        },
      };
      const result = HybridSearchUpdateMessageSchema.safeParse(input);
      expect(result.success).toBe(true);
    });

    it('parses a LEAVE change type', () => {
      const input = {
        type: 'HYBRID_SEARCH_UPDATE',
        payload: {
          subscriptionId: 'sub-1',
          key: 'prod-42',
          score: 0,
          methodScores: {},
          changeType: 'LEAVE',
        },
      };
      const result = HybridSearchUpdateMessageSchema.safeParse(input);
      expect(result.success).toBe(true);
    });
  });

  describe('HybridSearchUnsubMessageSchema', () => {
    it('parses a well-formed HYBRID_SEARCH_UNSUB message', () => {
      const input = {
        type: 'HYBRID_SEARCH_UNSUB',
        payload: {
          subscriptionId: 'sub-1',
        },
      };
      const result = HybridSearchUnsubMessageSchema.safeParse(input);
      expect(result.success).toBe(true);
    });
  });
});
