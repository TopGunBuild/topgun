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

