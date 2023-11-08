import {defineConfig} from 'tsup';

export default defineConfig((options) => {
    return {
        minify: !options.watch,
        sourcemap: options.watch,
        treeshake: true,
        globalName: 'TopGun',
        splitting: true,
        bundle: true,
        clean: true,
        dts: true,
        format: ['cjs', 'esm', 'iife'],
        outDir: 'dist',
        entry: [
            'src/index.ts',
            'src/**/index.ts',
        ],
        define: {
            global: 'window'
        }
    }
})