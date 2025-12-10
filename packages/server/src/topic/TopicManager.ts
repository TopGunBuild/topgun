import { ClusterManager } from '../cluster/ClusterManager';
import { logger } from '../utils/logger';

export interface TopicManagerConfig {
  cluster: ClusterManager;
  /** Callback to send message to a specific client */
  sendToClient: (clientId: string, message: any) => void;
}

export class TopicManager {
  private subscribers: Map<string, Set<string>> = new Map(); // topic -> Set<clientId>
  private cluster: ClusterManager;
  private sendToClient: (clientId: string, message: any) => void;
  private readonly MAX_SUBSCRIPTIONS = 100; // M1: Basic limit

  constructor(config: TopicManagerConfig) {
    this.cluster = config.cluster;
    this.sendToClient = config.sendToClient;
  }

  private validateTopic(topic: string): void {
    // H2: Validation
    if (!topic || topic.length > 256 || !/^[\w\-.:/]+$/.test(topic)) {
      throw new Error('Invalid topic name');
    }
  }

  /**
   * Subscribe a client to a topic
   */
  public subscribe(clientId: string, topic: string) {
    this.validateTopic(topic);

    // Check limit (M1)
    // This is expensive (iterating all topics). Optimized: maintain client->topics map?
    // For now, iterate.
    let count = 0;
    for (const subs of this.subscribers.values()) {
        if (subs.has(clientId)) count++;
    }
    if (count >= this.MAX_SUBSCRIPTIONS) {
        throw new Error('Subscription limit reached');
    }

    if (!this.subscribers.has(topic)) {
      this.subscribers.set(topic, new Set());
    }
    this.subscribers.get(topic)!.add(clientId);
    logger.debug({ clientId, topic }, 'Client subscribed to topic');
  }

  /**
   * Unsubscribe a client from a topic
   */
  public unsubscribe(clientId: string, topic: string) {
    const subs = this.subscribers.get(topic);
    if (subs) {
      subs.delete(clientId);
      if (subs.size === 0) {
        this.subscribers.delete(topic);
      }
      logger.debug({ clientId, topic }, 'Client unsubscribed from topic');
    }
  }

  /**
   * Clean up all subscriptions for a client (e.g. on disconnect)
   */
  public unsubscribeAll(clientId: string) {
    for (const [topic, subs] of this.subscribers) {
      if (subs.has(clientId)) {
        subs.delete(clientId);
        if (subs.size === 0) {
          this.subscribers.delete(topic);
        }
      }
    }
  }

  /**
   * Publish a message to a topic
   * @param topic Topic name
   * @param data Message data
   * @param senderId Client ID of the publisher (optional)
   * @param fromCluster Whether this message came from another cluster node
   */
  public publish(topic: string, data: any, senderId?: string, fromCluster: boolean = false) {
    this.validateTopic(topic);

    // 1. Send to local subscribers
    const subs = this.subscribers.get(topic);
    if (subs) {
        const payload = {
            topic,
            data,
            publisherId: senderId,
            timestamp: Date.now()
        };
        
        const message = {
            type: 'TOPIC_MESSAGE',
            payload
        };

        for (const clientId of subs) {
            // Don't echo back to sender if local
            if (clientId !== senderId) {
                this.sendToClient(clientId, message);
            }
        }
    }

    // 2. Broadcast to cluster (only if not already from cluster)
    if (!fromCluster) {
        this.cluster.getMembers().forEach(nodeId => {
            if (!this.cluster.isLocal(nodeId)) {
                this.cluster.send(nodeId, 'CLUSTER_TOPIC_PUB', {
                    topic,
                    data,
                    originalSenderId: senderId
                });
            }
        });
    }
  }
}

