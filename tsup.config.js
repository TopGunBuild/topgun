import {defineConfig} from 'tsup';

export default defineConfig((options) => {
    return {
        minify: !options.watch,
        sourcemap: options.watch,
        globalName: 'TopGun',
        splitting: true,
        bundle: true,
        clean: true,
        dts: true,
        format: ['cjs', 'esm', 'iife'],
        entryPoints: {
            client: 'src/client/index.ts',
            crdt: 'src/crdt/index.ts',
            server: 'src/server/index.ts',
            stream: 'src/stream/index.ts',
            storage: 'src/storage/index.ts',
            sea: 'src/sea/index.ts',
            utils: 'src/utils/index.ts',
            logger: 'src/logger/index.ts',
            types: 'src/types/index.ts'
        },
        define: {
            global: 'window'
        }
    }
})