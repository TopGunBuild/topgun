import { defineConfig, type Options } from 'tsup';

export default defineConfig((options: Options) => ({
    entry    : {
        client: 'src/client/index.ts',
        server: 'src/server/index.ts',
        index : 'src/index.ts',
    },
    treeshake: true,
    clean    : true,
    dts      : true,
    minify   : true,
    format   : ['cjs', 'esm'],
    define   : {
        global: 'window',
    },
    ...options,
}));
