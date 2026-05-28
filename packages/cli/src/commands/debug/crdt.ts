import chalk from 'chalk';
import fs from 'fs';

const NOT_AVAILABLE_MSG =
  'Not available: Rust server does not expose debug endpoints yet. Track progress in the debug endpoints roadmap.';

// Actions that require server-side debug endpoints (not yet implemented in Rust server)
const NETWORK_ACTIONS = ['export', 'stats', 'conflicts', 'timeline', 'tail'];

interface CrdtOptions {
  map?: string;
  output?: string;
  input?: string;
  format?: string;
  interval?: string;
  limit?: string;
}

interface Operation {
  timestamp: { millis: number };
  operation: string;
  key?: string;
  nodeId: string;
}

async function debugCrdt(action: string, options: CrdtOptions) {
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

  if (NETWORK_ACTIONS.includes(action)) {
    console.log(chalk.yellow(`\n  ${NOT_AVAILABLE_MSG}\n`));
    return;
  }

  // replay is purely local — reads a JSON file, no server needed
  if (action === 'replay') {
    await replayOperations(options);
  }
}

async function replayOperations(options: CrdtOptions) {
  if (!options.input) {
    console.error(chalk.red('\n  --input <file> is required'));
    process.exit(1);
  }

  console.log(chalk.bold('\n  Replaying operations\n'));

  try {
    const data = fs.readFileSync(options.input, 'utf8');
    const history = JSON.parse(data) as { operations?: Operation[] };

    const operations = history.operations || [];
    console.log(chalk.gray(`  Loaded ${operations.length} operations`));

    if (operations.length === 0) {
      console.log(chalk.yellow('  No operations to replay'));
      console.log('');
      return;
    }

    const limit = parseInt(options.limit || '20');
    const toShow = operations.slice(-limit);

    console.log(chalk.gray(`\n  Showing last ${toShow.length} operations:\n`));

    for (let i = 0; i < toShow.length; i++) {
      const op = toShow[i];
      const time = new Date(op.timestamp.millis).toISOString();
      const opColor =
        op.operation === 'set' ? chalk.green : op.operation === 'delete' ? chalk.red : chalk.yellow;

      console.log(
        chalk.gray(`  ${i + 1}.`) +
          chalk.gray(` [${time}]`) +
          opColor(` ${op.operation.toUpperCase()}`) +
          chalk.white(` ${op.key || ''}`) +
          chalk.gray(` (${op.nodeId})`),
      );
    }

    console.log(chalk.green('\n  Replay complete'));
  } catch (error) {
    console.error(chalk.red(`  Error: ${(error as Error).message}`));
    process.exit(1);
  }

  console.log('');
}

export default debugCrdt;
