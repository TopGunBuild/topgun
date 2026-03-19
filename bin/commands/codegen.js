#!/usr/bin/env node
'use strict';

/**
 * CLI handler for the `topgun codegen` command.
 * Reads a topgun.schema.ts file and generates JSON + TypeScript output files.
 */
module.exports = function codegen(options) {
  // Register ts-node so that require() can load .ts schema files
  try {
    require('ts-node').register({
      transpileOnly: true,
      compilerOptions: {
        module: 'commonjs',
        esModuleInterop: true,
      },
    });
  } catch (_err) {
    // ts-node not available — schema file must already be compiled to JS
  }

  const { runCodegen } = require('@topgunbuild/schema');

  const schemaPath = options.schema || './topgun.schema.ts';
  const outDir = options.outDir || './generated';
  const typescript = options.typescript !== false;
  const json = options.json !== false;

  console.log(`Running codegen...`);
  console.log(`  Schema: ${schemaPath}`);
  console.log(`  Output: ${outDir}`);

  try {
    runCodegen({ schemaPath, outDir, typescript, json });
    console.log('Done.');
  } catch (err) {
    console.error(`Codegen failed: ${err.message}`);
    process.exit(1);
  }
};
