const chalk = require('chalk');

const NOT_AVAILABLE_MSG =
  'Not available: Rust server does not expose debug endpoints yet. Track progress in the debug endpoints roadmap.';

module.exports = async function searchExplain(options) {
  console.log(chalk.bold('\n  TopGun Search Explainer\n'));
  console.log(chalk.yellow(`  ${NOT_AVAILABLE_MSG}\n`));
};
