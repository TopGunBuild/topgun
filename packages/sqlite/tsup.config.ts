import { defineConfig, type Options } from 'tsup';

export default defineConfig((options: Options) => ({
    entry : {
        browser: 'src/browser/index.ts',
        node   : 'src/node/index.ts',
        index  : 'src/index.ts',
    },
    clean : true,
    dts   : true,
    minify: true,
    format: ['cjs', 'esm'],
    ...options,
}));
