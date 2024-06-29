import { defineConfig, type Options } from 'tsup';

export default defineConfig((options: Options) => ({
    entry     : ['src/index.ts'],
    splitting : true,
    bundle    : true,
    clean     : true,
    dts       : true,
    minify    : true,
    format    : ['cjs', 'esm', 'iife'],
    globalName: 'TopGun',
    metafile  : true,
    define    : {
        global: 'window',
    },
    ...options,
}));
