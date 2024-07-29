import { defineConfig, type Options } from 'tsup';

export default defineConfig((options: Options) => ({
    entry    : ['src/textencoder.ts', 'src/textencoder.browser.ts'],
    treeshake: true,
    dts      : true,
    minify   : true,
    format   : ['cjs', 'esm'],
    ...options,
}));
