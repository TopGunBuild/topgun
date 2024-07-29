import { defineConfig, type Options } from 'tsup';
import nodePolyfills from 'rollup-plugin-polyfill-node';

export default defineConfig((options: Options) => ({
    entry     : ['src/index.ts'],
    treeshake : true,
    dts       : true,
    minify    : true,
    format    : ['cjs', 'esm', 'iife'],
    globalName: 'TopGun',
    metafile  : true,
    define    : {
        global: 'window',
    },
    plugins: [
        // nodePolyfills( /* options */ )
    ],
    ...options,
}));
