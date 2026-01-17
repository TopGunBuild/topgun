const chalk = require('chalk');

/**
 * Search Explain Command (Phase 14C)
 *
 * Explains search results with BM25/RRF score breakdown.
 */
module.exports = async function searchExplain(options) {
  const serverUrl = process.env.TOPGUN_SERVER_URL || 'http://localhost:9091';

  console.log(chalk.bold('\n  TopGun Search Explainer\n'));

  if (!options.query) {
    // Show help if no query
    console.log(chalk.gray('  Usage:'));
    console.log(chalk.gray('    topgun search:explain --query "search terms" --map mapName'));
    console.log('');
    console.log(chalk.gray('  Options:'));
    console.log(chalk.gray('    --query <query>    Search query (required)'));
    console.log(chalk.gray('    --map <name>       Map name to search'));
    console.log(chalk.gray('    --limit <n>        Max results to show (default: 10)'));
    console.log(chalk.gray('    --verbose          Show detailed breakdown for each result'));
    console.log('');
    console.log(chalk.gray('  Examples:'));
    console.log(chalk.gray('    topgun search:explain --query "john doe" --map users'));
    console.log(chalk.gray('    topgun search:explain --query "react hooks" --map docs --verbose'));
    console.log('');
    console.log(chalk.yellow('  Note: Server must be running with TOPGUN_DEBUG=true'));
    console.log('');
    return;
  }

  try {
    const response = await fetch(`${serverUrl}/debug/search/explain`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        query: options.query,
        mapId: options.map,
        limit: parseInt(options.limit) || 10,
      }),
    });

    if (!response.ok) {
      const text = await response.text();
      console.error(chalk.red(`  Failed to get search debug info: ${response.status}`));
      console.error(chalk.gray(`  ${text}`));
      process.exit(1);
    }

    const debugInfo = await response.json();

    if (debugInfo.error) {
      console.log(chalk.yellow(`  ${debugInfo.error}`));
      console.log('');
      return;
    }

    // Print query summary
    console.log(chalk.white(`  Query: "${debugInfo.query}"`));
    console.log(chalk.gray(`  Tokens: ${debugInfo.queryTokens?.join(', ') || 'N/A'}`));
    console.log(chalk.gray(`  Type: ${debugInfo.searchType || 'N/A'}`));
    console.log(chalk.gray(`  Results: ${debugInfo.matchingDocuments || 0} of ${debugInfo.totalDocuments || 0} documents`));
    console.log('');

    // Print timing
    if (debugInfo.timing) {
      console.log(chalk.white('  Timing:'));
      console.log(chalk.gray(`    Tokenization: ${debugInfo.timing.tokenization?.toFixed(2) || 0}ms`));
      console.log(chalk.gray(`    Index lookup: ${debugInfo.timing.indexLookup?.toFixed(2) || 0}ms`));
      console.log(chalk.gray(`    Scoring: ${debugInfo.timing.scoring?.toFixed(2) || 0}ms`));
      console.log(chalk.gray(`    Ranking: ${debugInfo.timing.ranking?.toFixed(2) || 0}ms`));
      if (debugInfo.timing.fusion !== undefined) {
        console.log(chalk.gray(`    Fusion: ${debugInfo.timing.fusion?.toFixed(2) || 0}ms`));
      }
      console.log(chalk.gray(`    Total: ${debugInfo.timing.total?.toFixed(2) || 0}ms`));
      console.log('');
    }

    // Print index stats
    if (debugInfo.indexStats) {
      console.log(chalk.white('  Index stats:'));
      console.log(chalk.gray(`    Type: ${debugInfo.indexStats.indexType || 'N/A'}`));
      console.log(chalk.gray(`    Size: ${debugInfo.indexStats.indexSize || 0} entries`));
      console.log(chalk.gray(`    Terms searched: ${debugInfo.indexStats.termsSearched || 0}`));
      console.log('');
    }

    // Print results with score breakdown
    if (debugInfo.results && debugInfo.results.length > 0) {
      console.log(chalk.white('  Results:\n'));

      for (let i = 0; i < debugInfo.results.length; i++) {
        const result = debugInfo.results[i];

        console.log(chalk.cyan(`  ${i + 1}. ${result.docId}`));
        console.log(chalk.white(`     Final score: ${result.finalScore?.toFixed(4) || 0}`));

        if (result.scoreBreakdown?.bm25) {
          const bm25 = result.scoreBreakdown.bm25;
          console.log(chalk.gray(`     BM25: ${bm25.score?.toFixed(4) || 0}`));

          if (options.verbose && bm25.matchedTerms) {
            for (const term of bm25.matchedTerms.slice(0, 3)) {
              const tf = bm25.tf?.[term] || 0;
              const idf = bm25.idf?.[term] || 0;
              console.log(chalk.gray(`       - "${term}": TF=${tf.toFixed(3)}, IDF=${idf.toFixed(3)}`));
            }
          }
        }

        if (result.scoreBreakdown?.exact) {
          const exact = result.scoreBreakdown.exact;
          console.log(chalk.gray(`     Exact: ${exact.score?.toFixed(4) || 0} (${exact.matchedFields?.join(', ') || 'N/A'})`));
        }

        if (result.scoreBreakdown?.rrf) {
          console.log(chalk.gray(`     RRF rank: ${result.scoreBreakdown.rrf.rank || 0}`));

          if (options.verbose && result.scoreBreakdown.rrf.contributingRanks) {
            for (const contrib of result.scoreBreakdown.rrf.contributingRanks) {
              console.log(chalk.gray(`       - ${contrib.source}: rank ${contrib.rank}`));
            }
          }
        }

        if (result.scoreBreakdown?.vector) {
          const vector = result.scoreBreakdown.vector;
          console.log(chalk.gray(`     Vector: ${vector.score?.toFixed(4) || 0} (${vector.similarity || 'N/A'})`));
        }

        console.log('');
      }
    } else {
      console.log(chalk.yellow('  No results found'));
      console.log('');
    }
  } catch (error) {
    console.error(chalk.red(`  Connection error: ${error.message}`));
    console.log(chalk.gray('\n  Make sure the server is running with TOPGUN_DEBUG=true'));
    process.exit(1);
  }
};
