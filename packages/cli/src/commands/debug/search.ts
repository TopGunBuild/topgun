import chalk from 'chalk';

const NOT_AVAILABLE_MSG =
  'Not available: Rust server does not expose debug endpoints yet. Track progress in the debug endpoints roadmap.';

interface SearchExplainOptions {
  query?: string;
  map?: string;
  limit?: string;
  verbose?: boolean;
}

// eslint-disable-next-line @typescript-eslint/no-unused-vars
async function searchExplain(_options: SearchExplainOptions) {
  console.log(chalk.bold('\n  TopGun Search Explainer\n'));
  console.log(chalk.yellow(`  ${NOT_AVAILABLE_MSG}\n`));
}

export default searchExplain;
