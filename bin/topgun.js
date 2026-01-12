#!/usr/bin/env node

const { Command } = require('commander');
const path = require('path');
const pkg = require('../package.json');

const program = new Command();

program
  .name('topgun')
  .description('TopGun CLI - Unified developer experience')
  .version(pkg.version);

// Core commands
program
  .command('doctor')
  .description('Check environment setup')
  .action(require('./commands/doctor'));

program
  .command('setup')
  .description('Interactive project setup')
  .option('-y, --yes', 'Skip prompts, use defaults')
  .option('--storage <type>', 'Storage backend: sqlite, postgres, memory', 'sqlite')
  .action(require('./commands/setup'));

program
  .command('dev')
  .description('Start development server')
  .option('--no-db', 'Skip database startup')
  .option('-p, --port <port>', 'Server port', '8080')
  .action(require('./commands/dev'));

program
  .command('test [scope]')
  .description('Run tests (core, client, server, e2e, k6:smoke)')
  .option('--coverage', 'Generate coverage report')
  .action(require('./commands/test'));

program
  .command('config')
  .description('Manage configuration')
  .option('--transport <type>', 'Set transport: ws, http')
  .option('--storage <type>', 'Set storage: sqlite, postgres, memory')
  .option('--show', 'Show current configuration')
  .action(require('./commands/config'));

// Cluster commands
program
  .command('cluster:start')
  .description('Start local cluster')
  .option('-n, --nodes <count>', 'Number of nodes', '3')
  .action(require('./commands/cluster/start'));

program
  .command('cluster:stop')
  .description('Stop local cluster')
  .action(require('./commands/cluster/stop'));

program
  .command('cluster:status')
  .description('Show cluster status')
  .action(require('./commands/cluster/status'));

// Debug commands (placeholders for Part C)
program
  .command('debug:crdt <action>')
  .description('CRDT debugging tools (export, replay, diff)')
  .option('--map <name>', 'Map name')
  .option('--output <file>', 'Output file')
  .action(require('./commands/debug/crdt'));

program
  .command('search:explain')
  .description('Explain search results')
  .option('--query <query>', 'Search query')
  .option('--map <name>', 'Map name')
  .action(require('./commands/debug/search'));

program.parse();
