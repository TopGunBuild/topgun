(async () => {
    const esbuild = require('esbuild');
    const fs = require('fs');
    const metafile = false;

    // Automatically exclude all node_modules from the bundled version
    const {nodeExternalsPlugin} = require('esbuild-node-externals');

    let result = await esbuild.build({
        entryPoints: ['src/socket-server/build-api.ts'],
        outfile: 'dist/node/server.js',
        bundle: true,
        sourcemap: true,
        minify: false,
        platform: 'node',
        target: ['node14'],
        plugins: [nodeExternalsPlugin()],
        metafile
    });

    if (metafile) {
        let text = await esbuild.analyzeMetafile(result.metafile);
        fs.writeFileSync('dist/client.meta.json', JSON.stringify(result.metafile));
        console.log(text);
    }
})();
