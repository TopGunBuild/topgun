/**
 * Mock ClusterManager for unit tests.
 * Provides simulated cluster communication without actual network connections.
 */

import { EventEmitter } from 'events';

export class MockClusterManager extends EventEmitter {
  config = { nodeId: 'node-1' };
  private members: string[] = ['node-1', 'node-2', 'node-3'];
  private sentMessages: Array<{ nodeId: string; type: string; payload: any }> = [];

  constructor(nodeId: string = 'node-1', members: string[] = ['node-1', 'node-2', 'node-3']) {
    super();
    this.config = { nodeId };
    this.members = members;
  }

  getMembers(): string[] {
    return this.members;
  }

  setMembers(members: string[]): void {
    this.members = members;
  }

  send(nodeId: string, type: string, payload: any): void {
    this.sentMessages.push({ nodeId, type, payload });
  }

  getSentMessages(): Array<{ nodeId: string; type: string; payload: any }> {
    return this.sentMessages;
  }

  clearSentMessages(): void {
    this.sentMessages = [];
  }

  /**
   * Simulate receiving a message from another node.
   * This triggers the 'message' event that the DistributedSubscriptionCoordinator listens to.
   */
  receiveMessage(senderId: string, type: string, payload: any): void {
    this.emit('message', { type, senderId, payload });
  }
}
