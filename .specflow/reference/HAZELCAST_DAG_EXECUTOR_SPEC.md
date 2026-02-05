# DAG Executor Specification for TopGun

> **Session Date:** 2026-01-12
> **Purpose:** Specification for implementing Hazelcast-style DAG executor in TopGun
> **Priority:** High (main gap identified in architecture comparison)

---

## 1. Overview

The DAG (Directed Acyclic Graph) executor is the core component for distributed query execution. It transforms query plans into executable graphs that can be distributed across cluster nodes.

### 1.1 Why DAG Executor?

Without DAG executor, TopGun cannot:
- Execute distributed GROUP BY
- Perform multi-partition JOINs
- Run parallel query execution across cluster
- Optimize data shuffling between nodes

### 1.2 Hazelcast Reference

```java
// Hazelcast DAG.java
public class DAG implements IdentifiedDataSerializable {
    private final Set<Edge> edges = new LinkedHashSet<>();
    private final Map<String, Vertex> nameToVertex = new HashMap<>();

    Vertex newVertex(String name, ProcessorMetaSupplier metaSupplier);
    DAG edge(Edge edge);
    Iterator<Vertex> iterator(); // topological sort
}
```

---

## 2. Core Data Structures

### 2.1 Vertex

```typescript
/**
 * A vertex in the DAG represents a processing stage.
 * Each vertex has a processor supplier that creates actual processors.
 */
interface Vertex {
  /** Unique name within the DAG */
  name: string;

  /** Number of parallel processors on each node */
  localParallelism: number;

  /** Factory for creating processors */
  processorSupplier: ProcessorSupplier;

  /** Optional: preferred partitions for this vertex */
  preferredPartitions?: number[];
}

/**
 * Factory for creating Processor instances.
 * Serializable for distribution to cluster nodes.
 */
interface ProcessorSupplier {
  /** Called on each node to create processors */
  get(count: number): Processor[];

  /** Serialization support */
  serialize(): Uint8Array;
  static deserialize(data: Uint8Array): ProcessorSupplier;
}

/**
 * Single-threaded execution unit.
 * Processes items from inbox and emits to outbox.
 */
interface Processor {
  /** Initialize processor with context */
  init(context: ProcessorContext): void;

  /** Process items from inbox, emit to outbox */
  process(ordinal: number, inbox: Inbox): boolean;

  /** Complete processing (called when all inputs done) */
  complete(): boolean;

  /** Whether processor can share thread with others */
  isCooperative(): boolean;

  /** Cleanup resources */
  close(): void;
}
```

### 2.2 Edge

```typescript
/**
 * An edge connects two vertices and defines data flow.
 */
interface Edge {
  /** Source vertex name */
  sourceName: string;

  /** Source output ordinal (for vertices with multiple outputs) */
  sourceOrdinal: number;

  /** Destination vertex name */
  destName: string;

  /** Destination input ordinal */
  destOrdinal: number;

  /** How data is routed between processors */
  routingPolicy: RoutingPolicy;

  /** Optional: partitioner for PARTITIONED routing */
  partitioner?: Partitioner;

  /** Priority for scheduling (higher = earlier) */
  priority: number;
}

/**
 * Routing policies for edges.
 */
enum RoutingPolicy {
  /** Round-robin distribution to all consumers */
  UNICAST = 'unicast',

  /** Route by partition key */
  PARTITIONED = 'partitioned',

  /** Send to all consumers */
  BROADCAST = 'broadcast',

  /** 1:1 mapping between source and dest processors */
  ISOLATED = 'isolated',

  /** Local round-robin, remote broadcast */
  FANOUT = 'fanout',
}

/**
 * Determines partition for a data item.
 */
interface Partitioner {
  getPartition(item: unknown): number;
}
```

### 2.3 DAG

```typescript
/**
 * Directed Acyclic Graph for query execution.
 */
class DAG {
  private vertices: Map<string, Vertex> = new Map();
  private edges: Set<Edge> = new Set();
  private edgesBySource: Map<string, Edge[]> = new Map();
  private edgesByDest: Map<string, Edge[]> = new Map();

  /**
   * Add a new vertex to the DAG.
   */
  newVertex(name: string, supplier: ProcessorSupplier, parallelism = 1): Vertex {
    if (this.vertices.has(name)) {
      throw new Error(`Vertex '${name}' already exists`);
    }

    const vertex: Vertex = {
      name,
      localParallelism: parallelism,
      processorSupplier: supplier,
    };

    this.vertices.set(name, vertex);
    return vertex;
  }

  /**
   * Add an edge between two vertices.
   */
  edge(from: string | Vertex, to: string | Vertex, config?: Partial<Edge>): this {
    const sourceName = typeof from === 'string' ? from : from.name;
    const destName = typeof to === 'string' ? to : to.name;

    if (!this.vertices.has(sourceName)) {
      throw new Error(`Source vertex '${sourceName}' not found`);
    }
    if (!this.vertices.has(destName)) {
      throw new Error(`Destination vertex '${destName}' not found`);
    }

    const edge: Edge = {
      sourceName,
      sourceOrdinal: config?.sourceOrdinal ?? 0,
      destName,
      destOrdinal: config?.destOrdinal ?? 0,
      routingPolicy: config?.routingPolicy ?? RoutingPolicy.UNICAST,
      partitioner: config?.partitioner,
      priority: config?.priority ?? 0,
    };

    this.edges.add(edge);

    // Update indexes
    if (!this.edgesBySource.has(sourceName)) {
      this.edgesBySource.set(sourceName, []);
    }
    this.edgesBySource.get(sourceName)!.push(edge);

    if (!this.edgesByDest.has(destName)) {
      this.edgesByDest.set(destName, []);
    }
    this.edgesByDest.get(destName)!.push(edge);

    return this;
  }

  /**
   * Validate DAG and return vertices in topological order.
   */
  validate(): Vertex[] {
    // Check for cycles
    const visited = new Set<string>();
    const recursionStack = new Set<string>();

    const hasCycle = (name: string): boolean => {
      visited.add(name);
      recursionStack.add(name);

      const outEdges = this.edgesBySource.get(name) ?? [];
      for (const edge of outEdges) {
        if (!visited.has(edge.destName)) {
          if (hasCycle(edge.destName)) return true;
        } else if (recursionStack.has(edge.destName)) {
          return true;
        }
      }

      recursionStack.delete(name);
      return false;
    };

    for (const name of this.vertices.keys()) {
      if (!visited.has(name) && hasCycle(name)) {
        throw new Error('DAG contains a cycle');
      }
    }

    // Topological sort (Kahn's algorithm)
    const inDegree = new Map<string, number>();
    for (const name of this.vertices.keys()) {
      inDegree.set(name, 0);
    }
    for (const edge of this.edges) {
      inDegree.set(edge.destName, inDegree.get(edge.destName)! + 1);
    }

    const queue: string[] = [];
    for (const [name, degree] of inDegree) {
      if (degree === 0) queue.push(name);
    }

    const sorted: Vertex[] = [];
    while (queue.length > 0) {
      const name = queue.shift()!;
      sorted.push(this.vertices.get(name)!);

      for (const edge of this.edgesBySource.get(name) ?? []) {
        const newDegree = inDegree.get(edge.destName)! - 1;
        inDegree.set(edge.destName, newDegree);
        if (newDegree === 0) {
          queue.push(edge.destName);
        }
      }
    }

    if (sorted.length !== this.vertices.size) {
      throw new Error('DAG validation failed');
    }

    return sorted;
  }

  /**
   * Get all vertices.
   */
  getVertices(): Vertex[] {
    return Array.from(this.vertices.values());
  }

  /**
   * Get all edges.
   */
  getEdges(): Edge[] {
    return Array.from(this.edges);
  }

  /**
   * Serialize DAG for distribution.
   */
  serialize(): Uint8Array {
    // Use msgpackr for serialization
    const data = {
      vertices: Array.from(this.vertices.entries()).map(([name, v]) => ({
        name,
        localParallelism: v.localParallelism,
        processorSupplier: v.processorSupplier.serialize(),
      })),
      edges: Array.from(this.edges),
    };
    return serialize(data);
  }
}
```

---

## 3. Execution Components

### 3.1 Inbox/Outbox

```typescript
/**
 * Inbox provides items for processing.
 */
interface Inbox {
  /** Check if inbox is empty */
  isEmpty(): boolean;

  /** Peek at next item without removing */
  peek(): unknown | null;

  /** Remove and return next item */
  poll(): unknown | null;

  /** Remove all items and process with callback */
  drain(callback: (item: unknown) => void): void;

  /** Number of items available */
  size(): number;
}

/**
 * Outbox for emitting processed items.
 */
interface Outbox {
  /** Emit item to specific ordinal */
  offer(ordinal: number, item: unknown): boolean;

  /** Emit item to all ordinals */
  offerToAll(item: unknown): boolean;

  /** Check if outbox has capacity */
  hasCapacity(ordinal: number): boolean;

  /** Get number of ordinals */
  bucketCount(): number;
}
```

### 3.2 Processor Context

```typescript
/**
 * Context provided to processors during initialization.
 */
interface ProcessorContext {
  /** Node ID where processor is running */
  nodeId: string;

  /** Global processor index */
  globalProcessorIndex: number;

  /** Local processor index on this node */
  localProcessorIndex: number;

  /** Total processor count globally */
  totalParallelism: number;

  /** DAG vertex name */
  vertexName: string;

  /** Partition service for routing */
  partitionService: PartitionService;

  /** Logger instance */
  logger: Logger;
}
```

### 3.3 Execution Plan

```typescript
/**
 * Serializable execution plan for distribution to nodes.
 */
interface ExecutionPlan {
  /** DAG to execute */
  dag: DAG;

  /** Partition assignment: nodeId -> partitionIds */
  partitionAssignment: Map<string, number[]>;

  /** Plan version for staleness detection */
  version: number;

  /** Query configuration */
  config: QueryConfig;

  /** Timestamp when plan was created */
  createdAt: number;
}

/**
 * Query configuration.
 */
interface QueryConfig {
  /** Query timeout in ms */
  timeout: number;

  /** Memory limit per node */
  memoryLimit: number;

  /** Whether to collect metrics */
  collectMetrics: boolean;

  /** Consistency level for distributed ops */
  consistency: ConsistencyLevel;
}
```

---

## 4. Standard Processors

### 4.1 Scan Processor

```typescript
/**
 * Scans data from LWWMap/ORMap.
 */
class ScanProcessor implements Processor {
  private mapName: string;
  private predicate?: (key: string, value: unknown) => boolean;
  private context!: ProcessorContext;
  private iterator?: Iterator<[string, unknown]>;

  constructor(mapName: string, predicate?: (key: string, value: unknown) => boolean) {
    this.mapName = mapName;
    this.predicate = predicate;
  }

  init(context: ProcessorContext): void {
    this.context = context;
    // Get iterator for local partitions only
    this.iterator = this.getLocalDataIterator();
  }

  process(ordinal: number, inbox: Inbox): boolean {
    // Scan is a source - inbox is empty
    const outbox = this.getOutbox();
    let count = 0;
    const BATCH_SIZE = 1000;

    while (count < BATCH_SIZE) {
      const next = this.iterator?.next();
      if (!next || next.done) {
        return true; // Done
      }

      const [key, value] = next.value;
      if (!this.predicate || this.predicate(key, value)) {
        if (!outbox.offer(0, { key, value })) {
          // Backpressure - yield and retry
          return false;
        }
      }
      count++;
    }

    return false; // More to process
  }

  isCooperative(): boolean {
    return true;
  }
}
```

### 4.2 Filter Processor

```typescript
/**
 * Filters items based on predicate.
 */
class FilterProcessor implements Processor {
  private predicate: (item: unknown) => boolean;

  constructor(predicate: (item: unknown) => boolean) {
    this.predicate = predicate;
  }

  process(ordinal: number, inbox: Inbox): boolean {
    const outbox = this.getOutbox();

    inbox.drain((item) => {
      if (this.predicate(item)) {
        outbox.offer(0, item);
      }
    });

    return true;
  }

  isCooperative(): boolean {
    return true;
  }
}
```

### 4.3 Project Processor

```typescript
/**
 * Projects (transforms) items.
 */
class ProjectProcessor implements Processor {
  private projection: (item: unknown) => unknown;

  constructor(projection: (item: unknown) => unknown) {
    this.projection = projection;
  }

  process(ordinal: number, inbox: Inbox): boolean {
    const outbox = this.getOutbox();

    inbox.drain((item) => {
      outbox.offer(0, this.projection(item));
    });

    return true;
  }

  isCooperative(): boolean {
    return true;
  }
}
```

### 4.4 Aggregate Processor

```typescript
/**
 * Aggregates items (GROUP BY).
 */
class AggregateProcessor implements Processor {
  private keyExtractor: (item: unknown) => string;
  private aggregator: Aggregator;
  private groups: Map<string, AggregatorState> = new Map();

  constructor(keyExtractor: (item: unknown) => string, aggregator: Aggregator) {
    this.keyExtractor = keyExtractor;
    this.aggregator = aggregator;
  }

  process(ordinal: number, inbox: Inbox): boolean {
    inbox.drain((item) => {
      const key = this.keyExtractor(item);
      let state = this.groups.get(key);
      if (!state) {
        state = this.aggregator.createState();
        this.groups.set(key, state);
      }
      this.aggregator.accumulate(state, item);
    });

    return true;
  }

  complete(): boolean {
    const outbox = this.getOutbox();

    for (const [key, state] of this.groups) {
      const result = this.aggregator.finish(state);
      outbox.offer(0, { key, result });
    }

    return true;
  }

  isCooperative(): boolean {
    return true;
  }
}

interface Aggregator {
  createState(): AggregatorState;
  accumulate(state: AggregatorState, item: unknown): void;
  combine(state1: AggregatorState, state2: AggregatorState): AggregatorState;
  finish(state: AggregatorState): unknown;
}
```

### 4.5 Network Processors

```typescript
/**
 * Sends items to remote nodes.
 */
class NetworkSenderProcessor implements Processor {
  private targetNode: string;
  private clusterManager: ClusterManager;
  private buffer: unknown[] = [];
  private readonly BATCH_SIZE = 100;

  process(ordinal: number, inbox: Inbox): boolean {
    inbox.drain((item) => {
      this.buffer.push(item);

      if (this.buffer.length >= this.BATCH_SIZE) {
        this.flush();
      }
    });

    return true;
  }

  private flush(): void {
    if (this.buffer.length === 0) return;

    this.clusterManager.send(this.targetNode, 'DAG_DATA', {
      items: this.buffer,
    });

    this.buffer = [];
  }

  isCooperative(): boolean {
    return false; // Network I/O
  }
}

/**
 * Receives items from remote nodes.
 */
class NetworkReceiverProcessor implements Processor {
  private receiveBuffer: unknown[] = [];

  process(ordinal: number, inbox: Inbox): boolean {
    const outbox = this.getOutbox();

    // Drain network buffer
    while (this.receiveBuffer.length > 0) {
      const item = this.receiveBuffer.shift()!;
      if (!outbox.offer(0, item)) {
        this.receiveBuffer.unshift(item);
        return false;
      }
    }

    return true;
  }

  // Called by network layer
  onReceive(items: unknown[]): void {
    this.receiveBuffer.push(...items);
  }

  isCooperative(): boolean {
    return false; // Network I/O
  }
}
```

---

## 5. DAG Executor

```typescript
/**
 * Executes a DAG on the local node.
 */
class DAGExecutor {
  private dag: DAG;
  private context: ExecutionContext;
  private processors: Map<string, Processor[]> = new Map();
  private queues: Map<string, Queue<unknown>[]> = new Map();
  private completed: Set<string> = new Set();

  constructor(dag: DAG, context: ExecutionContext) {
    this.dag = dag;
    this.context = context;
  }

  /**
   * Initialize all processors.
   */
  async init(): Promise<void> {
    const vertices = this.dag.validate();

    for (const vertex of vertices) {
      const processors = vertex.processorSupplier.get(vertex.localParallelism);

      for (let i = 0; i < processors.length; i++) {
        processors[i].init({
          nodeId: this.context.nodeId,
          globalProcessorIndex: i,
          localProcessorIndex: i,
          totalParallelism: vertex.localParallelism,
          vertexName: vertex.name,
          partitionService: this.context.partitionService,
          logger: this.context.logger,
        });
      }

      this.processors.set(vertex.name, processors);
    }

    // Initialize queues for edges
    for (const edge of this.dag.getEdges()) {
      const key = `${edge.sourceName}:${edge.sourceOrdinal}`;
      if (!this.queues.has(key)) {
        this.queues.set(key, []);
      }
    }
  }

  /**
   * Execute DAG until completion.
   */
  async execute(): Promise<void> {
    const vertices = this.dag.validate();

    while (this.completed.size < vertices.length) {
      let madeProgress = false;

      for (const vertex of vertices) {
        if (this.completed.has(vertex.name)) continue;

        const processors = this.processors.get(vertex.name)!;
        let allDone = true;

        for (const processor of processors) {
          if (processor.isCooperative()) {
            // Run in current thread
            const done = this.runProcessor(processor, vertex.name);
            if (!done) allDone = false;
            madeProgress = true;
          } else {
            // Offload to worker pool
            await this.context.workerPool.submit({
              id: `${vertex.name}-${Date.now()}`,
              type: 'processor',
              payload: { processor, vertexName: vertex.name },
              priority: 'normal',
            });
          }
        }

        if (allDone) {
          this.completed.add(vertex.name);
        }
      }

      if (!madeProgress) {
        // All blocked - yield
        await new Promise(resolve => setImmediate(resolve));
      }
    }
  }

  private runProcessor(processor: Processor, vertexName: string): boolean {
    // Get inbox from upstream queues
    const inbox = this.createInbox(vertexName);

    // Run processor
    return processor.process(0, inbox);
  }
}
```

---

## 6. Integration with TopGun

### 6.1 Query to DAG Conversion

```typescript
/**
 * Converts QueryPlan to DAG.
 */
class QueryToDAGConverter {
  convert(plan: QueryPlan, mapName: string): DAG {
    const dag = new DAG();

    // Create source vertex
    const source = dag.newVertex('source', new ScanProcessorSupplier(mapName));

    // Process plan steps
    let lastVertex = source;
    lastVertex = this.processStep(dag, plan.root, lastVertex);

    // Add sink vertex
    const sink = dag.newVertex('sink', new CollectorProcessorSupplier());
    dag.edge(lastVertex, sink);

    return dag;
  }

  private processStep(dag: DAG, step: PlanStep, upstream: Vertex): Vertex {
    switch (step.type) {
      case 'filter': {
        const filter = dag.newVertex(
          `filter-${Date.now()}`,
          new FilterProcessorSupplier(step.predicate)
        );
        dag.edge(upstream, filter);
        return filter;
      }

      case 'index-scan': {
        // Index scan replaces source
        return dag.newVertex(
          `index-scan-${Date.now()}`,
          new IndexScanProcessorSupplier(step.index, step.query)
        );
      }

      case 'intersection': {
        // Multiple index scans + merge
        const scans = step.steps.map((s, i) =>
          this.processStep(dag, s, upstream)
        );
        const merge = dag.newVertex(
          `intersection-${Date.now()}`,
          new IntersectionProcessorSupplier()
        );
        for (const scan of scans) {
          dag.edge(scan, merge);
        }
        return merge;
      }

      // ... other step types

      default:
        return upstream;
    }
  }
}
```

### 6.2 Cluster Query Coordinator

```typescript
/**
 * Coordinates distributed query execution.
 */
class ClusterQueryCoordinator {
  private clusterManager: ClusterManager;
  private partitionService: PartitionService;

  /**
   * Execute query across cluster.
   */
  async executeQuery(
    query: Query,
    mapName: string,
    options: QueryOptions
  ): Promise<QueryResult> {
    // 1. Optimize query
    const plan = this.optimizer.optimize(query);

    // 2. Convert to DAG
    const dag = this.converter.convert(plan, mapName);

    // 3. Create execution plan
    const execPlan: ExecutionPlan = {
      dag,
      partitionAssignment: this.partitionService.getPartitionAssignment(),
      version: this.partitionService.getVersion(),
      config: {
        timeout: options.timeout ?? 30000,
        memoryLimit: options.memoryLimit ?? 100 * 1024 * 1024,
        collectMetrics: true,
        consistency: options.consistency ?? ConsistencyLevel.EVENTUAL,
      },
      createdAt: Date.now(),
    };

    // 4. Distribute plan to relevant nodes
    const involvedNodes = this.getInvolvedNodes(execPlan);
    await this.distributePlan(execPlan, involvedNodes);

    // 5. Execute and collect results
    return this.executeAndCollect(execPlan, involvedNodes);
  }

  private getInvolvedNodes(plan: ExecutionPlan): string[] {
    // Determine which nodes have relevant partitions
    const nodes = new Set<string>();

    for (const [nodeId, partitions] of plan.partitionAssignment) {
      if (partitions.length > 0) {
        nodes.add(nodeId);
      }
    }

    return Array.from(nodes);
  }
}
```

---

## 7. Testing Strategy

### 7.1 Unit Tests

```typescript
describe('DAG', () => {
  it('should detect cycles', () => {
    const dag = new DAG();
    dag.newVertex('a', mockSupplier());
    dag.newVertex('b', mockSupplier());
    dag.edge('a', 'b');
    dag.edge('b', 'a');

    expect(() => dag.validate()).toThrow('cycle');
  });

  it('should return topological order', () => {
    const dag = new DAG();
    dag.newVertex('source', mockSupplier());
    dag.newVertex('filter', mockSupplier());
    dag.newVertex('sink', mockSupplier());
    dag.edge('source', 'filter');
    dag.edge('filter', 'sink');

    const order = dag.validate();
    expect(order.map(v => v.name)).toEqual(['source', 'filter', 'sink']);
  });
});
```

### 7.2 Integration Tests

```typescript
describe('DAGExecutor', () => {
  it('should execute simple scan-filter-collect pipeline', async () => {
    const dag = new DAG();
    dag.newVertex('source', new ScanProcessorSupplier('testMap'));
    dag.newVertex('filter', new FilterProcessorSupplier(x => x.age > 18));
    dag.newVertex('sink', new CollectorProcessorSupplier());
    dag.edge('source', 'filter');
    dag.edge('filter', 'sink');

    const executor = new DAGExecutor(dag, context);
    await executor.init();
    await executor.execute();

    const results = executor.getResults();
    expect(results.every(r => r.age > 18)).toBe(true);
  });
});
```

---

## 8. Performance Considerations

### 8.1 Batch Processing

- Process items in batches (default: 1000)
- Reduces overhead of per-item processing

### 8.2 Backpressure

- Outbox.offer() returns false when full
- Processor yields and retries later

### 8.3 Cooperative vs Non-cooperative

- Cooperative: share thread, return in ~1ms
- Non-cooperative: dedicated thread for blocking ops

### 8.4 Memory Management

- Use BufferPool for temporary allocations
- Limit in-flight items per queue

---

## 9. Future Enhancements

1. **Checkpointing:** Save DAG state for fault tolerance
2. **Streaming:** Continuous query execution
3. **Window Functions:** Time-based aggregations
4. **Spilling:** Disk-based overflow for large aggregations
5. **Metrics:** Detailed execution statistics

---

## References

- Hazelcast DAG: `hazelcast/src/main/java/com/hazelcast/jet/core/DAG.java`
- Hazelcast ExecutionPlan: `hazelcast/src/main/java/com/hazelcast/jet/impl/execution/init/ExecutionPlan.java`
- Hazelcast Processor: `hazelcast/src/main/java/com/hazelcast/jet/core/Processor.java`
