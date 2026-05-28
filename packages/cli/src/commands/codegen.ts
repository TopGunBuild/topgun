interface CodegenOptions {
  schema?: string;
  outDir?: string;
  typescript?: boolean;
  json?: boolean;
}

function codegen(options: CodegenOptions) {
  // Register ts-node so that require() can load .ts schema files at runtime
  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    require('ts-node').register({
      transpileOnly: true,
      compilerOptions: {
        module: 'commonjs',
        esModuleInterop: true,
      },
    });
  } catch {
    // ts-node not available — schema file must already be compiled to JS
  }

  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { runCodegen } = require('@topgunbuild/schema') as {
    runCodegen: (opts: {
      schemaPath: string;
      outDir: string;
      typescript: boolean;
      json: boolean;
    }) => void;
  };

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
    console.error(`Codegen failed: ${(err as Error).message}`);
    process.exit(1);
  }
}

export default codegen;
