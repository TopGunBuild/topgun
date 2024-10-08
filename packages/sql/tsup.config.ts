import { defineConfig, type Options } from 'tsup';

export default defineConfig((options: Options) => ({
    entry    : {
        browser: 'src/browser.ts',
        index  : 'src/index.ts',
    },
    treeshake: true,
    dts      : true,
    minify   : true,
    format   : ['cjs', 'esm'],
    ...options,
}));
