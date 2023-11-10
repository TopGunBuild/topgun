const fs = require('fs');
const path = require('path');
const PACKAGE_DIRS = require('./package-dirs');

const getContent = function (name)
{
    return `{
    "name": "@topgunbuild/topgun/${name}",
    "types": "../dist/${name}/index.d.ts",
    "main": "../dist/${name}/index.js",
    "module": "../dist/${name}/index.mjs",
    "sideEffects": false
}`;
};

for (const dir of PACKAGE_DIRS)
{
    const folderPatch = path.join(__dirname, '..', dir);

    if (!fs.existsSync(folderPatch))
    {
        fs.mkdirSync(folderPatch);
        fs.writeFileSync(path.join(folderPatch, 'package.json'), getContent(dir));
    }
}
