const chalk = require('chalk');
const fs = require('fs');
const path = require('path');

/**
 * CRDT Debug Command (Phase 14C)
 *
 * Actions:
 * - export: Export operation history for a map
 * - stats: Show CRDT statistics
 * - conflicts: Show resolved conflicts
 * - timeline: Display operation timeline
 * - replay: Replay operations step by step
 * - tail: Watch live operations (requires WebSocket)
 */
module.exports = async function debugCrdt(action, options) {
  const serverUrl = process.env.TOPGUN_SERVER_URL || 'http://localhost:9090';

  const validActions = ['export', 'stats', 'conflicts', 'timeline', 'replay', 'tail'];

  if (!validActions.includes(action)) {
    console.log(chalk.red(`\n  Unknown action: ${action}`));
    console.log(chalk.gray(`  Valid actions: ${validActions.join(', ')}`));
    console.log('');
    console.log(chalk.white('  Examples:'));
    console.log(chalk.gray('    topgun debug:crdt export --map users --output ops.json'));
    console.log(chalk.gray('    topgun debug:crdt stats --map users'));
    console.log(chalk.gray('    topgun debug:crdt conflicts --map users'));
    console.log(chalk.gray('    topgun debug:crdt timeline --map users'));
    console.log(chalk.gray('    topgun debug:crdt replay --input ops.json'));
    console.log(chalk.gray('    topgun debug:crdt tail --map users'));
    console.log('');
    process.exit(1);
  }

  switch (action) {
    case 'export':
      await exportHistory(serverUrl, options);
      break;

    case 'stats':
      await showStatistics(serverUrl, options);
      break;

    case 'conflicts':
      await showConflicts(serverUrl, options);
      break;

    case 'timeline':
      await showTimeline(serverUrl, options);
      break;

    case 'replay':
      await replayOperations(options);
      break;

    case 'tail':
      await tailOperations(serverUrl, options);
      break;
  }
};

async function exportHistory(serverUrl, options) {
  console.log(chalk.bold('\n  Exporting CRDT history\n'));

  try {
    const response = await fetch(`${serverUrl}/debug/crdt/export`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        mapId: options.map,
        format: options.format || 'json',
      }),
    });

    if (!response.ok) {
      const text = await response.text();
      console.error(chalk.red(`  Failed to export history: ${response.status}`));
      console.error(chalk.gray(`  ${text}`));
      process.exit(1);
    }

    const data = await response.text();

    if (options.output) {
      fs.writeFileSync(options.output, data);
      console.log(chalk.green(`  Exported to ${options.output}`));

      // Show quick stats
      try {
        const parsed = JSON.parse(data);
        if (parsed.statistics) {
          console.log(chalk.gray(`  Operations: ${parsed.statistics.totalOperations}`));
          console.log(chalk.gray(`  Conflicts: ${parsed.statistics.conflictsResolved}`));
        }
      } catch {
        // Not JSON, ignore
      }
    } else {
      console.log(data);
    }
  } catch (error) {
    console.error(chalk.red(`  Connection error: ${error.message}`));
    console.log(chalk.gray('\n  Make sure the server is running with CRDT_DEBUG=true'));
    process.exit(1);
  }

  console.log('');
}

async function showStatistics(serverUrl, options) {
  console.log(chalk.bold('\n  CRDT Statistics\n'));

  try {
    const response = await fetch(`${serverUrl}/debug/crdt/stats`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ mapId: options.map }),
    });

    if (!response.ok) {
      console.error(chalk.red(`  Failed to get statistics: ${response.status}`));
      process.exit(1);
    }

    const stats = await response.json();

    console.log(chalk.white('  Total operations:'), chalk.cyan(stats.totalOperations));
    console.log(chalk.white('  Conflicts resolved:'), chalk.cyan(stats.conflictsResolved));
    console.log(chalk.white('  Unique keys:'), chalk.cyan(stats.uniqueKeys));
    console.log(chalk.white('  Avg ops/sec:'), chalk.cyan(stats.averageOpsPerSecond.toFixed(2)));
    console.log('');

    if (Object.keys(stats.operationsByType).length > 0) {
      console.log(chalk.white('  Operations by type:'));
      for (const [type, count] of Object.entries(stats.operationsByType)) {
        console.log(chalk.gray(`    - ${type}: ${count}`));
      }
      console.log('');
    }

    if (Object.keys(stats.operationsByNode).length > 0) {
      console.log(chalk.white('  Operations by node:'));
      for (const [node, count] of Object.entries(stats.operationsByNode)) {
        console.log(chalk.gray(`    - ${node}: ${count}`));
      }
    }
  } catch (error) {
    console.error(chalk.red(`  Connection error: ${error.message}`));
    console.log(chalk.gray('\n  Make sure the server is running with CRDT_DEBUG=true'));
    process.exit(1);
  }

  console.log('');
}

async function showConflicts(serverUrl, options) {
  console.log(chalk.bold('\n  CRDT Conflicts\n'));

  try {
    const response = await fetch(`${serverUrl}/debug/crdt/conflicts`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ mapId: options.map }),
    });

    if (!response.ok) {
      console.error(chalk.red(`  Failed to get conflicts: ${response.status}`));
      process.exit(1);
    }

    const conflicts = await response.json();

    if (conflicts.length === 0) {
      console.log(chalk.green('  No conflicts detected'));
      console.log('');
      return;
    }

    console.log(chalk.yellow(`  Found ${conflicts.length} conflicts:\n`));

    for (const c of conflicts) {
      console.log(chalk.white(`  Key: ${c.key}`));
      console.log(chalk.green(`    Winner: ${c.winnerNodeId} @ ${c.winnerTimestamp.millis}`));
      console.log(chalk.red(`    Loser:  ${c.loserNodeId} @ ${c.loserTimestamp.millis}`));
      console.log(chalk.gray(`    Resolved at: ${c.resolvedAt}`));
      console.log('');
    }
  } catch (error) {
    console.error(chalk.red(`  Connection error: ${error.message}`));
    console.log(chalk.gray('\n  Make sure the server is running with CRDT_DEBUG=true'));
    process.exit(1);
  }
}

async function showTimeline(serverUrl, options) {
  console.log(chalk.bold('\n  CRDT Timeline\n'));

  try {
    const intervalMs = parseInt(options.interval) || 1000;
    const response = await fetch(`${serverUrl}/debug/crdt/timeline`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        mapId: options.map,
        intervalMs,
      }),
    });

    if (!response.ok) {
      console.error(chalk.red(`  Failed to get timeline: ${response.status}`));
      process.exit(1);
    }

    const timeline = await response.json();

    if (timeline.length === 0) {
      console.log(chalk.yellow('  No operations recorded'));
      console.log('');
      return;
    }

    console.log(chalk.gray(`  Interval: ${intervalMs}ms\n`));

    for (const bucket of timeline) {
      const time = new Date(bucket.timestamp).toISOString();
      const opsCount = bucket.operations.length;
      const bar = '█'.repeat(Math.min(opsCount, 50));

      console.log(chalk.gray(`  ${time}`) + chalk.cyan(` [${opsCount}] `) + chalk.blue(bar));
    }
  } catch (error) {
    console.error(chalk.red(`  Connection error: ${error.message}`));
    process.exit(1);
  }

  console.log('');
}

async function replayOperations(options) {
  if (!options.input) {
    console.error(chalk.red('\n  --input <file> is required'));
    process.exit(1);
  }

  console.log(chalk.bold('\n  Replaying operations\n'));

  try {
    const data = fs.readFileSync(options.input, 'utf8');
    const history = JSON.parse(data);

    const operations = history.operations || [];
    console.log(chalk.gray(`  Loaded ${operations.length} operations`));

    if (operations.length === 0) {
      console.log(chalk.yellow('  No operations to replay'));
      console.log('');
      return;
    }

    // Show operations step by step
    const limit = parseInt(options.limit) || 20;
    const toShow = operations.slice(-limit);

    console.log(chalk.gray(`\n  Showing last ${toShow.length} operations:\n`));

    for (let i = 0; i < toShow.length; i++) {
      const op = toShow[i];
      const time = new Date(op.timestamp.millis).toISOString();
      const opColor = op.operation === 'set' ? chalk.green :
                     op.operation === 'delete' ? chalk.red : chalk.yellow;

      console.log(
        chalk.gray(`  ${i + 1}.`) +
        chalk.gray(` [${time}]`) +
        opColor(` ${op.operation.toUpperCase()}`) +
        chalk.white(` ${op.key || ''}`) +
        chalk.gray(` (${op.nodeId})`)
      );
    }

    console.log(chalk.green('\n  Replay complete'));
  } catch (error) {
    console.error(chalk.red(`  Error: ${error.message}`));
    process.exit(1);
  }

  console.log('');
}

async function tailOperations(serverUrl, options) {
  console.log(chalk.bold('\n  Watching CRDT operations (Ctrl+C to stop)\n'));

  // For now, poll the operations endpoint
  // A future improvement would use WebSocket streaming

  let lastOpId = null;
  const pollInterval = 1000;

  console.log(chalk.gray(`  Polling ${serverUrl} every ${pollInterval}ms\n`));

  const poll = async () => {
    try {
      const response = await fetch(`${serverUrl}/debug/crdt/operations`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          mapId: options.map,
          limit: 10,
        }),
      });

      if (!response.ok) {
        return;
      }

      const operations = await response.json();

      for (const op of operations) {
        if (lastOpId && op.id === lastOpId) continue;
        if (lastOpId === null) {
          lastOpId = op.id;
          continue; // Skip initial batch
        }

        const time = new Date(op.timestamp.millis).toISOString();
        const opColor = op.operation === 'set' ? chalk.green :
                       op.operation === 'delete' ? chalk.red : chalk.yellow;

        console.log(
          chalk.gray(`[${time}]`) +
          opColor(` ${op.operation}`) +
          chalk.white(` ${op.key || ''}`) +
          chalk.gray(` (${op.nodeId})`)
        );

        lastOpId = op.id;
      }
    } catch {
      // Ignore errors during polling
    }
  };

  // Initial poll
  await poll();

  // Continue polling
  const interval = setInterval(poll, pollInterval);

  // Handle Ctrl+C
  process.on('SIGINT', () => {
    clearInterval(interval);
    console.log(chalk.gray('\n\n  Stopped watching'));
    process.exit(0);
  });
}
