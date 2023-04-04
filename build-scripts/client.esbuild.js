(async () => {
    const esbuild = require('esbuild');
    const fs = require('fs');
    const metafile = true;

    // iife
    let result = await esbuild.build({
        globalName: 'TopGun',
        entryPoints: ['src/client/build-api.ts'],
        outfile: 'dist/client.js',
        bundle: true,
        sourcemap: true,
        minify: true,
        format: 'iife',
        target: ['esnext'],
        define: {global: 'window'},
        metafile
    });

    // esm
    await esbuild.build({
        globalName: 'TopGun',
        entryPoints: ['src/client/build-api.ts'],
        outfile: 'dist/client.module.js',
        bundle: true,
        sourcemap: true,
        minify: true,
        format: 'esm',
        target: ['esnext'],
        define: {global: 'window'},
    });

    if (metafile) {
        let text = await esbuild.analyzeMetafile(result.metafile);
        fs.writeFileSync('dist/client.meta.json', JSON.stringify(result.metafile));
        console.log(text);
    }
})();
