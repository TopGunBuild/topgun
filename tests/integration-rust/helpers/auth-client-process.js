/**
 * Starts helpers/auth-client.ts in a process of its own: a TopGunClient whose
 * credentials the test drives by command, on an event loop of its own.
 *
 * It loads the workspace source the way helpers/client-subscriber-process.js
 * does: ts-node transpiles the TypeScript on load and tsconfig-paths resolves
 * `@topgunbuild/core` and `@topgunbuild/client` to the packages' `src`.
 */
const path = require('path');

const project = path.join(__dirname, '..', 'tsconfig.json');
require('ts-node').register({ project, transpileOnly: true });
const tsconfigPaths = require('tsconfig-paths');
const config = tsconfigPaths.loadConfig(path.dirname(project));
tsconfigPaths.register({ baseUrl: config.absoluteBaseUrl, paths: config.paths });

process.env.TOPGUN_AUTH_CLIENT_CHILD = '1';
require('./auth-client.ts');
