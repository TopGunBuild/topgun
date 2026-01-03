/**
 * FTS Tokenizer with Porter Stemming and Stopwords
 *
 * Provides text tokenization for BM25 full-text search.
 * Features:
 * - Unicode-aware word boundary detection
 * - English stopwords filtering (174 words)
 * - Porter stemming algorithm for word normalization
 * - Configurable min/max token length
 *
 * @module fts/Tokenizer
 */

import type { TokenizerOptions } from './types';

/**
 * English stopwords list (174 common words).
 * These words are filtered out during tokenization as they
 * don't contribute to search relevance.
 */
export const ENGLISH_STOPWORDS = new Set([
  // Articles
  'a',
  'an',
  'the',

  // Pronouns
  'i',
  'me',
  'my',
  'myself',
  'we',
  'our',
  'ours',
  'ourselves',
  'you',
  'your',
  'yours',
  'yourself',
  'yourselves',
  'he',
  'him',
  'his',
  'himself',
  'she',
  'her',
  'hers',
  'herself',
  'it',
  'its',
  'itself',
  'they',
  'them',
  'their',
  'theirs',
  'themselves',
  'what',
  'which',
  'who',
  'whom',
  'this',
  'that',
  'these',
  'those',

  // Auxiliary verbs
  'am',
  'is',
  'are',
  'was',
  'were',
  'be',
  'been',
  'being',
  'have',
  'has',
  'had',
  'having',
  'do',
  'does',
  'did',
  'doing',
  'will',
  'would',
  'shall',
  'should',
  'can',
  'could',
  'may',
  'might',
  'must',
  'ought',

  // Prepositions
  'about',
  'above',
  'across',
  'after',
  'against',
  'along',
  'among',
  'around',
  'at',
  'before',
  'behind',
  'below',
  'beneath',
  'beside',
  'between',
  'beyond',
  'by',
  'down',
  'during',
  'except',
  'for',
  'from',
  'in',
  'inside',
  'into',
  'near',
  'of',
  'off',
  'on',
  'onto',
  'out',
  'outside',
  'over',
  'past',
  'since',
  'through',
  'throughout',
  'to',
  'toward',
  'towards',
  'under',
  'underneath',
  'until',
  'up',
  'upon',
  'with',
  'within',
  'without',

  // Conjunctions
  'and',
  'but',
  'or',
  'nor',
  'so',
  'yet',
  'both',
  'either',
  'neither',
  'not',
  'only',
  'as',
  'if',
  'than',
  'when',
  'while',
  'although',
  'because',
  'unless',
  'whether',

  // Adverbs
  'here',
  'there',
  'where',
  'when',
  'how',
  'why',
  'all',
  'each',
  'every',
  'any',
  'some',
  'no',
  'none',
  'more',
  'most',
  'other',
  'such',
  'own',
  'same',
  'too',
  'very',
  'just',
  'also',
  'now',
  'then',
  'again',
  'ever',
  'once',

  // Misc
  'few',
  'many',
  'much',
  'several',
  's',
  't',
  'd',
  'll',
  'm',
  've',
  're',
]);

/**
 * Porter Stemming Algorithm
 *
 * Reduces English words to their stem (root form).
 * Based on the algorithm by Martin Porter (1980).
 *
 * @see https://tartarus.org/martin/PorterStemmer/
 *
 * @param word - Word to stem (should be lowercase)
 * @returns Stemmed word
 */
export function porterStem(word: string): string {
  if (!word || word.length < 3) {
    return word;
  }

  // Work with the word
  let stem = word;

  // Step 1a: Plurals
  if (stem.endsWith('sses')) {
    stem = stem.slice(0, -2);
  } else if (stem.endsWith('ies')) {
    stem = stem.slice(0, -2);
  } else if (!stem.endsWith('ss') && stem.endsWith('s')) {
    stem = stem.slice(0, -1);
  }

  // Step 1b: -ed and -ing
  const step1bRegex = /^(.+?)(eed|ed|ing)$/;
  const step1bMatch = stem.match(step1bRegex);

  if (step1bMatch) {
    const [, base, suffix] = step1bMatch;

    if (suffix === 'eed') {
      // Only remove if stem has measure > 0
      if (getMeasure(base) > 0) {
        stem = base + 'ee';
      }
    } else if (hasVowel(base)) {
      stem = base;

      // Additional processing after -ed/-ing removal
      if (stem.endsWith('at') || stem.endsWith('bl') || stem.endsWith('iz')) {
        stem = stem + 'e';
      } else if (endsWithDoubleConsonant(stem) && !stem.match(/[lsz]$/)) {
        stem = stem.slice(0, -1);
      } else if (getMeasure(stem) === 1 && endsWithCVC(stem)) {
        stem = stem + 'e';
      }
    }
  }

  // Step 1c: Terminal y
  if (stem.endsWith('y') && hasVowel(stem.slice(0, -1))) {
    stem = stem.slice(0, -1) + 'i';
  }

  // Step 2: Double suffixes
  const step2Suffixes: Array<[RegExp, string, number]> = [
    [/ational$/, 'ate', 0],
    [/tional$/, 'tion', 0],
    [/enci$/, 'ence', 0],
    [/anci$/, 'ance', 0],
    [/izer$/, 'ize', 0],
    [/abli$/, 'able', 0],
    [/alli$/, 'al', 0],
    [/entli$/, 'ent', 0],
    [/eli$/, 'e', 0],
    [/ousli$/, 'ous', 0],
    [/ization$/, 'ize', 0],
    [/ation$/, 'ate', 0],
    [/ator$/, 'ate', 0],
    [/alism$/, 'al', 0],
    [/iveness$/, 'ive', 0],
    [/fulness$/, 'ful', 0],
    [/ousness$/, 'ous', 0],
    [/aliti$/, 'al', 0],
    [/iviti$/, 'ive', 0],
    [/biliti$/, 'ble', 0],
  ];

  for (const [regex, replacement, minMeasure] of step2Suffixes) {
    if (regex.test(stem)) {
      const base = stem.replace(regex, '');
      if (getMeasure(base) > minMeasure) {
        stem = base + replacement;
        break;
      }
    }
  }

  // Step 3: -icate, -ful, -ness, etc.
  const step3Suffixes: Array<[RegExp, string, number]> = [
    [/icate$/, 'ic', 0],
    [/ative$/, '', 0],
    [/alize$/, 'al', 0],
    [/iciti$/, 'ic', 0],
    [/ical$/, 'ic', 0],
    [/ful$/, '', 0],
    [/ness$/, '', 0],
  ];

  for (const [regex, replacement, minMeasure] of step3Suffixes) {
    if (regex.test(stem)) {
      const base = stem.replace(regex, '');
      if (getMeasure(base) > minMeasure) {
        stem = base + replacement;
        break;
      }
    }
  }

  // Step 4: Final suffixes
  const step4Suffixes: Array<[RegExp, number]> = [
    [/al$/, 1],
    [/ance$/, 1],
    [/ence$/, 1],
    [/er$/, 1],
    [/ic$/, 1],
    [/able$/, 1],
    [/ible$/, 1],
    [/ant$/, 1],
    [/ement$/, 1],
    [/ment$/, 1],
    [/ent$/, 1],
    [/ion$/, 1],
    [/ou$/, 1],
    [/ism$/, 1],
    [/ate$/, 1],
    [/iti$/, 1],
    [/ous$/, 1],
    [/ive$/, 1],
    [/ize$/, 1],
  ];

  for (const [regex, minMeasure] of step4Suffixes) {
    if (regex.test(stem)) {
      const base = stem.replace(regex, '');
      if (getMeasure(base) > minMeasure) {
        // Special case for -ion (must be preceded by s or t)
        if (regex.source === 'ion$') {
          if (base.match(/[st]$/)) {
            stem = base;
          }
        } else {
          stem = base;
        }
        break;
      }
    }
  }

  // Step 5a: Final -e
  if (stem.endsWith('e')) {
    const base = stem.slice(0, -1);
    const measure = getMeasure(base);
    if (measure > 1 || (measure === 1 && !endsWithCVC(base))) {
      stem = base;
    }
  }

  // Step 5b: Double consonant
  if (getMeasure(stem) > 1 && endsWithDoubleConsonant(stem) && stem.endsWith('l')) {
    stem = stem.slice(0, -1);
  }

  return stem;
}

/**
 * Check if a character is a vowel.
 */
function isVowel(char: string, prevChar?: string): boolean {
  if ('aeiou'.includes(char)) {
    return true;
  }
  // Y is a vowel if preceded by a consonant
  if (char === 'y' && prevChar && !'aeiou'.includes(prevChar)) {
    return true;
  }
  return false;
}

/**
 * Check if a string contains a vowel.
 */
function hasVowel(str: string): boolean {
  for (let i = 0; i < str.length; i++) {
    if (isVowel(str[i], i > 0 ? str[i - 1] : undefined)) {
      return true;
    }
  }
  return false;
}

/**
 * Calculate the "measure" of a word (number of VC sequences).
 * [C](VC){m}[V] where m is the measure.
 */
function getMeasure(str: string): number {
  // Convert to CV pattern
  let pattern = '';
  for (let i = 0; i < str.length; i++) {
    pattern += isVowel(str[i], i > 0 ? str[i - 1] : undefined) ? 'v' : 'c';
  }

  // Count VC sequences
  const matches = pattern.match(/vc/g);
  return matches ? matches.length : 0;
}

/**
 * Check if word ends with a double consonant (e.g., -ll, -ss, -zz).
 */
function endsWithDoubleConsonant(str: string): boolean {
  if (str.length < 2) return false;
  const last = str[str.length - 1];
  const secondLast = str[str.length - 2];
  return last === secondLast && !'aeiou'.includes(last);
}

/**
 * Check if word ends with CVC pattern where last C is not w, x, or y.
 */
function endsWithCVC(str: string): boolean {
  if (str.length < 3) return false;

  const last3 = str.slice(-3);
  const c1 = !'aeiou'.includes(last3[0]);
  const v = isVowel(last3[1], last3[0]);
  const c2 = !'aeiou'.includes(last3[2]) && !'wxy'.includes(last3[2]);

  return c1 && v && c2;
}

/**
 * FTS Tokenizer
 *
 * Splits text into searchable tokens with normalization.
 *
 * @example
 * ```typescript
 * const tokenizer = new Tokenizer();
 * const tokens = tokenizer.tokenize('The quick brown foxes');
 * // ['quick', 'brown', 'fox']
 * ```
 */
export class Tokenizer {
  private readonly options: Required<TokenizerOptions>;

  /**
   * Create a new Tokenizer.
   *
   * @param options - Configuration options
   */
  constructor(options?: TokenizerOptions) {
    this.options = {
      lowercase: true,
      stopwords: ENGLISH_STOPWORDS,
      stemmer: porterStem,
      minLength: 2,
      maxLength: 40,
      ...options,
    };
  }

  /**
   * Tokenize text into an array of normalized tokens.
   *
   * @param text - Text to tokenize
   * @returns Array of tokens
   */
  tokenize(text: string): string[] {
    // Handle null/undefined/empty
    if (!text || typeof text !== 'string') {
      return [];
    }

    // 1. Lowercase if enabled
    let processed = this.options.lowercase ? text.toLowerCase() : text;

    // 2. Split on non-alphanumeric characters (Unicode-aware)
    // This regex matches Unicode letters and numbers
    const words = processed.split(/[^\p{L}\p{N}]+/u).filter((w) => w.length > 0);

    // 3. Filter and process each word
    const tokens: string[] = [];

    for (const word of words) {
      // Skip if too short before any processing
      if (word.length < this.options.minLength) {
        continue;
      }

      // Skip stopwords (before stemming)
      if (this.options.stopwords.has(word)) {
        continue;
      }

      // Apply stemmer
      const stemmed = this.options.stemmer(word);

      // Skip if too short after stemming
      if (stemmed.length < this.options.minLength) {
        continue;
      }

      // Skip if too long
      if (stemmed.length > this.options.maxLength) {
        continue;
      }

      tokens.push(stemmed);
    }

    return tokens;
  }
}
