import { BM25InvertedIndex } from './BM25InvertedIndex';
import type { SerializedIndex, TermInfo } from './types';

/**
 * Serializer for BM25InvertedIndex
 *
 * Handles serialization/deserialization of the inverted index for persistence.
 * Matches the structure defined in PHASE_11_IMPLEMENTATION_NOTES.
 */
export class IndexSerializer {
  /**
   * Serialize inverted index to a JSON-serializable object.
   * Note: In a real app, you might want to encoding this to binary (msgpack) later.
   */
  serialize(index: BM25InvertedIndex): SerializedIndex {
    const data: SerializedIndex = {
      version: 1,
      metadata: {
        totalDocs: index.getTotalDocs(),
        avgDocLength: index.getAvgDocLength(),
        createdAt: Date.now(),
        lastModified: Date.now(),
      },
      terms: this.serializeTerms(index),
      docLengths: this.serializeDocLengths(index),
    };

    return data;
  }

  /**
   * Deserialize from object into a new BM25InvertedIndex.
   */
  deserialize(data: SerializedIndex): BM25InvertedIndex {
    // Validate version
    if (data.version !== 1) {
      throw new Error(`Unsupported index version: ${data.version}`);
    }

    const index = new BM25InvertedIndex();
    this.loadIntoIndex(index, data);

    return index;
  }

  private serializeTerms(index: BM25InvertedIndex): SerializedIndex['terms'] {
    const terms: SerializedIndex['terms'] = [];
    const indexMap = (index as any).index as Map<string, TermInfo[]>; // Access private map

    // We need access to internal map.
    // Since we can't easily access private 'index' property without 'any' cast or getter,
    // we rely on iteration if available, or 'any' cast for this system component.
    // The public API getTerms() only returns keys.

    for (const term of index.getTerms()) {
      const termInfos = index.getDocumentsForTerm(term);
      terms.push({
        term,
        idf: index.getIDF(term),
        postings: termInfos.map((info) => ({
          docId: info.docId,
          termFrequency: info.termFrequency,
          positions: info.fieldPositions,
        })),
      });
    }

    return terms;
  }

  private serializeDocLengths(index: BM25InvertedIndex): Record<string, number> {
    const lengths: Record<string, number> = {};
    for (const [docId, length] of index.getDocLengths()) {
      lengths[docId] = length;
    }
    return lengths;
  }

  private loadIntoIndex(index: BM25InvertedIndex, data: SerializedIndex): void {
    // Restore metadata
    // We need to set private properties. 
    // We'll use a helper method on Index or 'any' cast for this serializer friend class.
    const idx = index as any;
    
    idx.totalDocs = data.metadata.totalDocs;
    idx.avgDocLength = data.metadata.avgDocLength;
    
    // Restore doc lengths
    idx.docLengths = new Map(Object.entries(data.docLengths));
    
    // Restore terms
    for (const { term, idf, postings } of data.terms) {
      const termInfos: TermInfo[] = postings.map((p) => ({
        docId: p.docId,
        termFrequency: p.termFrequency,
        fieldPositions: p.positions,
      }));
      
      idx.index.set(term, termInfos);
      idx.idfCache.set(term, idf);
      
      // We also need to restore docTerms for efficient removal
      // This is expensive to rebuild from inverted index (O(Terms * Docs)).
      // But essential for updates.
      for (const info of termInfos) {
        if (!idx.docTerms.has(info.docId)) {
          idx.docTerms.set(info.docId, new Set());
        }
        idx.docTerms.get(info.docId).add(term);
      }
    }
  }
}

