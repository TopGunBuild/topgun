import { defineConfig, type Options } from 'tsup';

export default defineConfig((options: Options) => ({
    entry    : ['src/time.ts', 'src/time.browser.ts'],
    treeshake: true,
    dts      : true,
    minify   : true,
    format   : ['cjs', 'esm'],
    ...options,
}));
