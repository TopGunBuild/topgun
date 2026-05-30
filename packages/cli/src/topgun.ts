import { Command } from 'commander';
import doctorCmd from './commands/doctor.js';
import setupCmd from './commands/setup.js';
import devCmd from './commands/dev.js';
import testCmd from './commands/test.js';
import configCmd from './commands/config.js';
import clusterStartCmd from './commands/cluster/start.js';
import clusterStopCmd from './commands/cluster/stop.js';
import clusterStatusCmd from './commands/cluster/status.js';
import codegenCmd from './commands/codegen.js';
import debugCrdtCmd from './commands/debug/crdt.js';
import searchExplainCmd from './commands/debug/search.js';
import * as dockerCmds from './commands/docker.js';

// Read the package version from this package's own manifest.
// After tsup builds to dist/, package.json is one directory up from dist/.
// eslint-disable-next-line @typescript-eslint/no-require-imports
const pkg = require('../package.json') as { version: string };

const program = new Command();

program
  .name('topgun')
  .description('TopGun CLI - Unified developer experience')
  .version(pkg.version);

// Core commands
program.command('doctor').description('Check environment setup').action(doctorCmd);

program
  .command('setup')
  .description('Interactive project setup')
  .option('-y, --yes', 'Skip prompts, use defaults')
  .action(setupCmd);

program
  .command('dev')
  .description('Start development server')
  .option('--no-db', 'Skip database startup')
  .option('-p, --port <port>', 'Server port', '8080')
  .option('--debug', 'Enable debug logging (RUST_LOG=debug, TOPGUN_LOG_FORMAT=json)')
  .option('--admin', 'Also start the admin dashboard (monorepo only)')
  .action(devCmd);

program
  .command('test [scope]')
  .description('Run tests (core, client, server, e2e, k6:smoke)')
  .option('--coverage', 'Generate coverage report')
  .action(testCmd);

program
  .command('config')
  .description('Manage configuration')
  .option('--storage <type>', 'Set storage: redb, postgres, null')
  .option('--show', 'Show current configuration')
  .action(configCmd);

// Cluster commands
program
  .command('cluster:start')
  .description('Start local cluster via Docker Compose')
  .action(clusterStartCmd);

program.command('cluster:stop').description('Stop local cluster').action(clusterStopCmd);

program.command('cluster:status').description('Show cluster status').action(clusterStatusCmd);

program
  .command('codegen')
  .description('Generate types and schema files from topgun.schema.ts')
  .option('--schema <path>', 'Path to schema file', './topgun.schema.ts')
  .option('--out-dir <dir>', 'Output directory', './generated')
  .option('--no-typescript', 'Skip TypeScript type generation')
  .option('--no-json', 'Skip JSON schema generation')
  .action(codegenCmd);

// Debug commands
program
  .command('debug:crdt <action>')
  .description('CRDT debugging tools (export, stats, conflicts, timeline, replay, tail)')
  .option('--map <name>', 'Map name to filter')
  .option('--output <file>', 'Output file for export')
  .option('--input <file>', 'Input file for replay')
  .option('--format <format>', 'Export format: json, csv, ndjson', 'json')
  .option('--interval <ms>', 'Timeline bucket interval in ms', '1000')
  .option('--limit <n>', 'Limit number of operations to show')
  .action(debugCrdtCmd);

program
  .command('search:explain')
  .description('Explain search results with BM25/RRF score breakdown')
  .option('--query <query>', 'Search query (required)')
  .option('--map <name>', 'Map name to search')
  .option('--limit <n>', 'Max results to show', '10')
  .option('--verbose', 'Show detailed breakdown for each result')
  .action(searchExplainCmd);

// Docker commands
program
  .command('docker:start')
  .description('Start Docker services')
  .option(
    '--with <profiles>',
    'Profiles to start (admin,monitoring,dbtools,k6,cluster,auto-setup,all)',
  )
  .action(dockerCmds.start);

program.command('docker:stop').description('Stop all Docker services').action(dockerCmds.stop);

program
  .command('docker:status')
  .description('Show Docker service status')
  .action(dockerCmds.status);

program
  .command('docker:logs')
  .description('Show Docker logs')
  .option('-s, --service <name>', 'Service name')
  .option('-f, --follow', 'Follow logs')
  .action(dockerCmds.logs);

program.parse();
