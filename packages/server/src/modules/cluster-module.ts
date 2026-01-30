import { ConsistencyLevel, DEFAULT_REPLICATION_CONFIG } from '@topgunbuild/core';
import { ClusterManager } from '../cluster/ClusterManager';
import { PartitionService } from '../cluster/PartitionService';
import { ReplicationPipeline } from '../cluster/ReplicationPipeline';
import { LockManager } from '../cluster/LockManager';
import { MerkleTreeManager } from '../cluster/MerkleTreeManager';
import { PartitionReassigner } from '../cluster/PartitionReassigner';
import { ReadReplicaHandler } from '../cluster/ReadReplicaHandler';
import { RepairScheduler } from '../cluster/RepairScheduler';
import type { ClusterModule, ClusterModuleConfig, ClusterModuleDeps } from './types';

export function createClusterModule(
  config: ClusterModuleConfig,
  deps: ClusterModuleDeps
): ClusterModule {
  const peers = config.resolvePeers ? config.resolvePeers() : (config.peers || []);

  const cluster = new ClusterManager({
    nodeId: config.nodeId,
    host: config.host || 'localhost',
    port: config.clusterPort ?? 0,
    peers,
    discovery: config.discovery,
    serviceName: config.serviceName,
    discoveryInterval: config.discoveryInterval,
    tls: config.clusterTls
  });

  const partitionService = new PartitionService(cluster);
  const lockManager = new LockManager();

  let replicationPipeline: ReplicationPipeline | undefined;
  if (config.replicationEnabled !== false) {
    replicationPipeline = new ReplicationPipeline(
      cluster,
      partitionService,
      {
        ...DEFAULT_REPLICATION_CONFIG,
        defaultConsistency: config.defaultConsistency ?? ConsistencyLevel.EVENTUAL,
        ...config.replicationConfig,
      }
    );
  }

  const merkleTreeManager = new MerkleTreeManager(config.nodeId);
  const partitionReassigner = new PartitionReassigner(cluster, partitionService, {
    reassignmentDelayMs: 1000
  });
  const readReplicaHandler = new ReadReplicaHandler(
    partitionService,
    cluster,
    config.nodeId,
    undefined,
    {
      defaultConsistency: config.defaultConsistency ?? ConsistencyLevel.STRONG,
      preferLocalReplica: true,
      loadBalancing: 'latency-based'
    }
  );
  const repairScheduler = new RepairScheduler(
    merkleTreeManager,
    cluster,
    partitionService,
    config.nodeId,
    { enabled: true, scanIntervalMs: 300000, maxConcurrentRepairs: 2 }
  );

  return {
    cluster,
    partitionService,
    replicationPipeline,
    lockManager,
    merkleTreeManager,
    partitionReassigner,
    readReplicaHandler,
    repairScheduler,
  };
}
