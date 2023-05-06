import {defineConfig} from 'tsup';

export default defineConfig((options) =>
{
    return {
        minify     : !options.watch,
        bundle     : !options.watch,
        sourcemap  : options.watch,
        globalName : 'TopGun',
        splitting  : false,
        clean      : true,
        dts        : true,
        format     : ['cjs', 'esm', 'iife'],
        entryPoints: {
            client: 'src/client/index.ts',
            server: 'src/server/index.ts'
        },
        define: {
            global: 'window'
        }
    }
})