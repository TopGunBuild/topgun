const fs = require('fs');
const path = require('path');

const PACKAGE_DIRS = ['client', 'server', 'stream', 'sea'];

const getContent = function (name)
{
    return `{
    "name": "topgun/${name}",
    "typings": "../dist/${name}.d.ts",
    "main": "../dist/${name}.js",
    "module": "../dist/${name}.mjs",
    "sideEffects": false
}`;
};

for (const dir of PACKAGE_DIRS)
{
    if (!fs.existsSync(dir))
    {
        fs.mkdirSync(dir);
        fs.writeFileSync(path.join(__dirname, '..', dir, 'package.json'), getContent(dir));
    }
}
