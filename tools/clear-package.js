const fs = require('fs');
const path = require('path');
const PACKAGE_DIRS = require('./package-dirs');

for (const dir of PACKAGE_DIRS)
{
    const folderPatch = path.join(__dirname, '..', dir);

    if (fs.existsSync(folderPatch))
    {
        fs.rmSync(path.join(folderPatch), { recursive: true, force: true });
    }
}
