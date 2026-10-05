// The default run: every test file except the ones listed in opt-in-suites.js.
const base = require('./jest.base.config');
const { excludedSuites, ignorePatterns, describeExclusions } = require('./opt-in-suites');

const excluded = excludedSuites();

// Say what was left out and why on every run, so a green result is never read
// as covering these files.
for (const line of describeExclusions(excluded)) {
  console.log(line);
}

/** @type {import('ts-jest').JestConfigWithTsJest} */
module.exports = {
  ...base,
  testPathIgnorePatterns: [...base.testPathIgnorePatterns, ...ignorePatterns(excluded)],
};
