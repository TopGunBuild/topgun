/**
 * Starts helpers/client-subscriber.ts in a process of its own: a TopGunClient
 * live subscriber that does not share the test's event loop.
 *
 * It runs the same workspace source the jest run maps: ts-node compiles the
 * TypeScript on load (transpile only, as ts-jest is configured to type-check
 * separately), and tsconfig-paths resolves `@topgunbuild/core` and
 * `@topgunbuild/client` through this directory's tsconfig to the packages'
 * `src`, so no package has to be built first.
 */
const path = require('path');

const project = path.join(__dirname, '..', 'tsconfig.json');
require('ts-node').register({ project, transpileOnly: true });
const tsconfigPaths = require('tsconfig-paths');
const config = tsconfigPaths.loadConfig(path.dirname(project));
tsconfigPaths.register({ baseUrl: config.absoluteBaseUrl, paths: config.paths });

require('./client-subscriber.ts');
