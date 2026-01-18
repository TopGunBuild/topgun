// Jest setup: Validate worker scripts are compiled
const fs = require('fs');
const path = require('path');

const workerScriptPath = path.join(__dirname, 'dist', 'workers', 'worker-scripts', 'base.worker.js');

if (!fs.existsSync(workerScriptPath)) {
  console.warn('\n[WARN] Worker scripts not compiled. Run "pnpm build" first for worker thread tests.\n');
  console.warn('Worker thread tests will be skipped until build completes.\n');
}
