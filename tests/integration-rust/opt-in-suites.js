// Test files the default run leaves out, each with the reason and the script
// that runs it. This is the only exclusion list: `pnpm test:integration-rust`
// reads it locally and in CI alike.
//
// The files listed here fail, never skip, when what they need is missing, so a
// default run that included them would be red on every clean checkout.
const optInSuites = [
  {
    files: ['map-name-upgrade', 'map-name-rollback'],
    reason:
      'each drives two server binaries and needs the older one by path (OLD_SERVER_BINARY, OLD_SERVER_COMMIT, NEW_SERVER_COMMIT)',
    script: 'pnpm test:integration-rust:two-binary',
  },
  {
    files: ['store-refusal-liveness'],
    reason:
      'it needs a server built with the fault-injection cargo feature (FAULT_SERVER_BINARY, FAULT_SERVER_COMMIT)',
    script: 'pnpm test:integration-rust:liveness',
  },
];

// Left out only where the environment asks for it. The 3-node suite fails to
// form a cluster in a fraction of runs, so the blocking CI job sets the
// variable and a separate non-blocking job runs the suite by name.
const envGatedSuites = [
  {
    files: ['cluster-routing'],
    env: 'INTEGRATION_RUST_SKIP_CLUSTER_ROUTING',
    reason: 'INTEGRATION_RUST_SKIP_CLUSTER_ROUTING is set; the suite is run on its own',
    script: 'pnpm test:integration-rust cluster-routing',
  },
];

function excludedSuites(env = process.env) {
  return [...optInSuites, ...envGatedSuites.filter((suite) => Boolean(env[suite.env]))];
}

function ignorePatterns(suites) {
  return suites.flatMap((suite) => suite.files.map((file) => `/${file}\\.test\\.ts$`));
}

function describeExclusions(suites) {
  return suites.map(
    (suite) =>
      `EXCLUDED from the default run: ${suite.files.join(', ')} — ${suite.reason}. Run with: ${suite.script}`,
  );
}

module.exports = { excludedSuites, ignorePatterns, describeExclusions };
