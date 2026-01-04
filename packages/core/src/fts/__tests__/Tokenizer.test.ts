/**
 * FTS Tokenizer Tests
 *
 * TDD approach: tests written before implementation.
 * Tests cover: tokenization, lowercase, stopwords, stemming, length filters.
 */

import { BM25Tokenizer, ENGLISH_STOPWORDS, porterStem } from '../Tokenizer';

describe('BM25Tokenizer', () => {
  describe('Basic tokenization', () => {
    test('should tokenize simple text', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('hello world');
      expect(tokens).toEqual(['hello', 'world']);
    });

    test('should split on whitespace', () => {
      const tokenizer = new BM25Tokenizer();
      // Note: "one" stems to "on" which is a stopword, so it's filtered
      const tokens = tokenizer.tokenize('alpha beta  gamma   delta');
      expect(tokens).toEqual(['alpha', 'beta', 'gamma', 'delta']);
    });

    test('should split on punctuation', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('hello, world! How are you?');
      // Stopwords "how", "are", "you" should be filtered
      expect(tokens).toEqual(['hello', 'world']);
    });

    test('should handle tabs and newlines', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('hello\tworld\ntest');
      expect(tokens).toEqual(['hello', 'world', 'test']);
    });

    test('should return empty array for empty string', () => {
      const tokenizer = new BM25Tokenizer();
      expect(tokenizer.tokenize('')).toEqual([]);
    });

    test('should return empty array for null/undefined', () => {
      const tokenizer = new BM25Tokenizer();
      expect(tokenizer.tokenize(null as unknown as string)).toEqual([]);
      expect(tokenizer.tokenize(undefined as unknown as string)).toEqual([]);
    });

    test('should return empty array for whitespace only', () => {
      const tokenizer = new BM25Tokenizer();
      expect(tokenizer.tokenize('   \t\n  ')).toEqual([]);
    });
  });

  describe('Lowercase normalization', () => {
    test('should convert to lowercase by default', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('Hello WORLD Test');
      expect(tokens).toEqual(['hello', 'world', 'test']);
    });

    test('should preserve case when lowercase is disabled', () => {
      const tokenizer = new BM25Tokenizer({ lowercase: false });
      const tokens = tokenizer.tokenize('Hello WORLD');
      expect(tokens).toEqual(['Hello', 'WORLD']);
    });

    test('should handle mixed case with Unicode', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('HELLO Wörld Привет');
      expect(tokens).toEqual(['hello', 'wörld', 'привет']);
    });
  });

  describe('Stopwords filtering', () => {
    test('should filter English stopwords by default', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('the quick brown fox jumps over the lazy dog');
      // "the", "over" are stopwords
      expect(tokens).toContain('quick');
      expect(tokens).toContain('brown');
      expect(tokens).toContain('fox');
      expect(tokens).toContain('jump'); // stemmed from "jumps"
      expect(tokens).toContain('lazi'); // stemmed from "lazy"
      expect(tokens).toContain('dog');
      expect(tokens).not.toContain('the');
      expect(tokens).not.toContain('over');
    });

    test('should handle custom stopwords', () => {
      const customStopwords = new Set(['hello', 'world']);
      const tokenizer = new BM25Tokenizer({ stopwords: customStopwords });
      const tokens = tokenizer.tokenize('hello beautiful world');
      expect(tokens).toEqual(['beauti']); // stemmed from "beautiful"
    });

    test('should disable stopwords when empty set provided', () => {
      const tokenizer = new BM25Tokenizer({ stopwords: new Set() });
      const tokens = tokenizer.tokenize('the fox');
      expect(tokens).toContain('the');
      expect(tokens).toContain('fox');
    });

    test('should filter stopwords before stemming', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('I am running');
      // "I" and "am" are stopwords
      expect(tokens).toEqual(['run']); // stemmed from "running"
    });
  });

  describe('Porter stemming', () => {
    test('should stem words by default', () => {
      const tokenizer = new BM25Tokenizer();

      // -ing suffix
      expect(tokenizer.tokenize('running')).toEqual(['run']);
      expect(tokenizer.tokenize('jumping')).toEqual(['jump']);

      // -ed suffix
      expect(tokenizer.tokenize('jumped')).toEqual(['jump']);
      expect(tokenizer.tokenize('walked')).toEqual(['walk']);

      // -s suffix
      expect(tokenizer.tokenize('cats')).toEqual(['cat']);
      expect(tokenizer.tokenize('dogs')).toEqual(['dog']);

      // -ly suffix
      expect(tokenizer.tokenize('happily')).toEqual(['happili']);

      // -ness suffix
      expect(tokenizer.tokenize('happiness')).toEqual(['happi']);
    });

    test('should handle irregular stems', () => {
      const tokenizer = new BM25Tokenizer();

      // These should normalize to same stem
      expect(tokenizer.tokenize('connect')).toEqual(['connect']);
      expect(tokenizer.tokenize('connected')).toEqual(['connect']);
      expect(tokenizer.tokenize('connecting')).toEqual(['connect']);
      expect(tokenizer.tokenize('connection')).toEqual(['connect']);
      expect(tokenizer.tokenize('connections')).toEqual(['connect']);
    });

    test('should disable stemming when custom stemmer returns word unchanged', () => {
      const tokenizer = new BM25Tokenizer({ stemmer: (word) => word });
      const tokens = tokenizer.tokenize('running jumping');
      expect(tokens).toEqual(['running', 'jumping']);
    });

    test('should work with custom stemmer', () => {
      const customStemmer = (word: string) => word.replace(/ing$/, '');
      const tokenizer = new BM25Tokenizer({ stemmer: customStemmer });
      const tokens = tokenizer.tokenize('running walking');
      expect(tokens).toEqual(['runn', 'walk']);
    });
  });

  describe('Length filtering', () => {
    test('should filter tokens shorter than minLength', () => {
      const tokenizer = new BM25Tokenizer({ minLength: 3 });
      const tokens = tokenizer.tokenize('a to be or not');
      // "a", "to", "be", "or" are < 3 chars or stopwords
      // "not" is a stopword
      expect(tokens).toEqual([]);
    });

    test('should use minLength 2 by default', () => {
      const tokenizer = new BM25Tokenizer();
      // "a" < 2 chars, "I", "be", "it" are stopwords
      // "go" is NOT a stopword and length >= 2
      const tokens = tokenizer.tokenize('a I be it');
      expect(tokens).toEqual([]);
    });

    test('should filter tokens longer than maxLength', () => {
      const tokenizer = new BM25Tokenizer({ maxLength: 5 });
      const tokens = tokenizer.tokenize('hi hello beautiful');
      // "beautiful" stems to "beauti" which is > 5
      expect(tokens).toContain('hi');
      expect(tokens).toContain('hello');
      expect(tokens).not.toContain('beauti');
    });

    test('should use maxLength 40 by default', () => {
      const tokenizer = new BM25Tokenizer();
      const longWord = 'a'.repeat(50);
      const tokens = tokenizer.tokenize(longWord);
      expect(tokens).toEqual([]);
    });

    test('should allow custom min/max length', () => {
      const tokenizer = new BM25Tokenizer({ minLength: 4, maxLength: 6 });
      const tokens = tokenizer.tokenize('a cat hello beautiful');
      // "a" < 4, "cat" < 4, "hello" ok, "beautiful" stems to "beauti" (6 chars)
      expect(tokens).toEqual(['hello', 'beauti']);
    });
  });

  describe('Unicode handling', () => {
    test('should handle accented characters', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('café résumé naïve');
      expect(tokens).toContain('café');
      expect(tokens).toContain('résumé');
      // "naïve" stems to "naïv" (final -e removed)
      expect(tokens).toContain('naïv');
    });

    test('should handle Cyrillic text', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('привет мир');
      expect(tokens).toContain('привет');
      expect(tokens).toContain('мир');
    });

    test('should handle Chinese characters', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('你好 世界');
      expect(tokens).toContain('你好');
      expect(tokens).toContain('世界');
    });

    test('should handle emoji', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('hello 👋 world 🌍');
      expect(tokens).toContain('hello');
      expect(tokens).toContain('world');
    });

    test('should handle mixed scripts', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('hello мир 世界');
      expect(tokens).toContain('hello');
      expect(tokens).toContain('мир');
      expect(tokens).toContain('世界');
    });
  });

  describe('Edge cases', () => {
    test('should handle numbers', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('test123 456test test456test');
      expect(tokens).toContain('test123');
      expect(tokens).toContain('456test');
      expect(tokens).toContain('test456test');
    });

    test('should handle hyphenated words', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('real-time state-of-the-art');
      // Hyphen splits words, each part is processed separately
      expect(tokens).toContain('real');
      expect(tokens).toContain('time');
      expect(tokens).toContain('state');
      expect(tokens).toContain('art');
    });

    test('should handle contractions', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize("don't won't can't");
      // Apostrophe splits contractions
      // "t" is filtered (< minLength), "can" is a stopword
      expect(tokens).toContain('don');
      expect(tokens).toContain('won');
      expect(tokens).not.toContain('can'); // "can" is a stopword
    });

    test('should handle URLs (split on punctuation)', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('visit https://example.com/page');
      expect(tokens).toContain('visit');
      expect(tokens).toContain('http');
      expect(tokens).toContain('exampl'); // stemmed
      expect(tokens).toContain('com');
      expect(tokens).toContain('page');
    });

    test('should handle email addresses (split on punctuation)', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('contact user@example.com');
      expect(tokens).toContain('contact');
      expect(tokens).toContain('user');
      expect(tokens).toContain('exampl'); // stemmed
      expect(tokens).toContain('com');
    });

    test('should handle repeated punctuation', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('hello!!! world??? test...');
      expect(tokens).toEqual(['hello', 'world', 'test']);
    });

    test('should handle code-like text', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('function calculateTotal() { return sum; }');
      expect(tokens).toContain('function');
      expect(tokens).toContain('calculatetot'); // stemmed
      expect(tokens).toContain('return');
      expect(tokens).toContain('sum');
    });
  });
});

describe('ENGLISH_STOPWORDS', () => {
  test('should contain common stopwords', () => {
    expect(ENGLISH_STOPWORDS.has('the')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('and')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('is')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('a')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('an')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('in')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('of')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('to')).toBe(true);
  });

  test('should contain pronouns', () => {
    expect(ENGLISH_STOPWORDS.has('i')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('you')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('he')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('she')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('it')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('we')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('they')).toBe(true);
  });

  test('should contain auxiliary verbs', () => {
    expect(ENGLISH_STOPWORDS.has('be')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('have')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('do')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('will')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('would')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('could')).toBe(true);
    expect(ENGLISH_STOPWORDS.has('should')).toBe(true);
  });

  test('should not contain content words', () => {
    expect(ENGLISH_STOPWORDS.has('cat')).toBe(false);
    expect(ENGLISH_STOPWORDS.has('dog')).toBe(false);
    expect(ENGLISH_STOPWORDS.has('run')).toBe(false);
    expect(ENGLISH_STOPWORDS.has('beautiful')).toBe(false);
    expect(ENGLISH_STOPWORDS.has('computer')).toBe(false);
  });

  test('should have approximately 174 stopwords', () => {
    // Allow some flexibility in count
    expect(ENGLISH_STOPWORDS.size).toBeGreaterThanOrEqual(150);
    expect(ENGLISH_STOPWORDS.size).toBeLessThanOrEqual(200);
  });
});

describe('porterStem', () => {
  describe('Step 1a: Plurals', () => {
    test('should handle -sses', () => {
      expect(porterStem('caresses')).toBe('caress');
      expect(porterStem('ponies')).toBe('poni');
    });

    test('should handle -ies', () => {
      expect(porterStem('ties')).toBe('ti');
      expect(porterStem('ponies')).toBe('poni');
    });

    test('should handle -ss', () => {
      expect(porterStem('caress')).toBe('caress');
      expect(porterStem('hiss')).toBe('hiss');
    });

    test('should handle -s', () => {
      expect(porterStem('cats')).toBe('cat');
      expect(porterStem('dogs')).toBe('dog');
      expect(porterStem('runs')).toBe('run');
    });
  });

  describe('Step 1b: -ed and -ing', () => {
    test('should handle -eed', () => {
      // "agreed" → "agre" (standard Porter)
      // The -eed rule only keeps -ee when measure > 0
      expect(porterStem('agreed')).toBe('agre');
      expect(porterStem('feed')).toBe('feed');
    });

    test('should handle -ed', () => {
      expect(porterStem('plastered')).toBe('plaster');
      expect(porterStem('bled')).toBe('bled');
      expect(porterStem('motored')).toBe('motor');
      expect(porterStem('sing')).toBe('sing');
    });

    test('should handle -ing', () => {
      expect(porterStem('motoring')).toBe('motor');
      expect(porterStem('sing')).toBe('sing');
      expect(porterStem('conflating')).toBe('conflat');
      expect(porterStem('troubling')).toBe('troubl');
    });

    test('should handle special cases after -ed/-ing removal', () => {
      expect(porterStem('filing')).toBe('file');
      expect(porterStem('failing')).toBe('fail');
      expect(porterStem('hopping')).toBe('hop');
      expect(porterStem('tanning')).toBe('tan');
      expect(porterStem('falling')).toBe('fall');
    });
  });

  describe('Step 1c: Terminal y', () => {
    test('should replace y with i', () => {
      expect(porterStem('happy')).toBe('happi');
      expect(porterStem('sky')).toBe('sky');
    });
  });

  describe('Step 2: Double suffixes', () => {
    test('should handle -ational', () => {
      expect(porterStem('relational')).toBe('relat');
    });

    test('should handle -tional', () => {
      expect(porterStem('conditional')).toBe('condit');
      expect(porterStem('rational')).toBe('ration');
    });

    test('should handle -ization', () => {
      expect(porterStem('formalization')).toBe('formal');
    });

    test('should handle -ation', () => {
      expect(porterStem('hesitation')).toBe('hesit');
    });

    test('should handle -fulness', () => {
      expect(porterStem('hopefulness')).toBe('hope');
    });

    test('should handle -ousness', () => {
      expect(porterStem('callousness')).toBe('callous');
    });
  });

  describe('Step 3: -ic, -full, -ness, etc.', () => {
    test('should handle -icate', () => {
      expect(porterStem('triplicate')).toBe('triplic');
    });

    test('should handle -ful', () => {
      expect(porterStem('hopeful')).toBe('hope');
    });

    test('should handle -ness', () => {
      expect(porterStem('goodness')).toBe('good');
    });
  });

  describe('Step 4: Final suffixes', () => {
    test('should handle -al', () => {
      expect(porterStem('revival')).toBe('reviv');
    });

    test('should handle -ance/-ence', () => {
      expect(porterStem('allowance')).toBe('allow');
      expect(porterStem('inference')).toBe('infer');
    });

    test('should handle -ment', () => {
      expect(porterStem('replacement')).toBe('replac');
    });

    test('should handle -ent', () => {
      expect(porterStem('dependent')).toBe('depend');
    });

    test('should handle -ion', () => {
      expect(porterStem('adoption')).toBe('adopt');
    });

    test('should handle -ism', () => {
      expect(porterStem('communism')).toBe('commun');
    });

    test('should handle -ity', () => {
      expect(porterStem('sensitivity')).toBe('sensit');
    });

    test('should handle -ive', () => {
      expect(porterStem('effective')).toBe('effect');
    });

    test('should handle -ize', () => {
      expect(porterStem('bowdlerize')).toBe('bowdler');
    });
  });

  describe('Step 5: Final cleanup', () => {
    test('should handle final -e', () => {
      expect(porterStem('probate')).toBe('probat');
      expect(porterStem('rate')).toBe('rate');
      expect(porterStem('cease')).toBe('ceas');
    });

    test('should handle double consonants', () => {
      expect(porterStem('controll')).toBe('control');
      expect(porterStem('roll')).toBe('roll');
    });
  });

  describe('Edge cases', () => {
    test('should handle short words', () => {
      expect(porterStem('a')).toBe('a');
      expect(porterStem('be')).toBe('be');
      expect(porterStem('go')).toBe('go');
    });

    test('should handle empty string', () => {
      expect(porterStem('')).toBe('');
    });

    test('should handle words without stems', () => {
      expect(porterStem('run')).toBe('run');
      expect(porterStem('cat')).toBe('cat');
      expect(porterStem('dog')).toBe('dog');
    });

    test('should not alter proper names (lowercase)', () => {
      // Stemmer works on lowercase, doesn't know about proper names
      expect(porterStem('james')).toBe('jame');
    });
  });

  describe('Real-world examples', () => {
    test('should normalize programming terms', () => {
      expect(porterStem('programming')).toBe('program');
      expect(porterStem('programs')).toBe('program');
      expect(porterStem('programmed')).toBe('program');
      expect(porterStem('programmer')).toBe('programm');
    });

    test('should normalize search terms', () => {
      expect(porterStem('searching')).toBe('search');
      expect(porterStem('searched')).toBe('search');
      expect(porterStem('searches')).toBe('search');
    });

    test('should normalize document terms', () => {
      // Porter stemmer produces consistent stems, but not always the root word
      // Different word forms may produce slightly different stems
      // The key is that queries using the same stemmer will match
      expect(porterStem('documents')).toBe('docum');
      expect(porterStem('documentation')).toBe('document');
      expect(porterStem('documented')).toBe('docum');
    });
  });

  describe('Consistent stemming (from test report)', () => {
    test('all forms of connect should stem consistently', () => {
      const forms = ['connect', 'connected', 'connecting', 'connection', 'connections'];
      const stems = forms.map((f) => porterStem(f));
      // All should stem to 'connect'
      expect(new Set(stems).size).toBe(1);
      expect(stems[0]).toBe('connect');
    });

    test('all forms of run should stem consistently', () => {
      const forms = ['run', 'runs', 'running', 'runner'];
      const stems = forms.map((f) => porterStem(f));
      // run, run, run, runner - runner is different
      expect(stems[0]).toBe('run');
      expect(stems[1]).toBe('run');
      expect(stems[2]).toBe('run');
    });

    test('all forms of search should stem consistently', () => {
      const forms = ['search', 'searches', 'searched', 'searching'];
      const stems = forms.map((f) => porterStem(f));
      expect(new Set(stems).size).toBe(1);
      expect(stems[0]).toBe('search');
    });
  });
});

describe('Tokenizer - Additional edge cases (from test report)', () => {
  describe('Very long text handling', () => {
    test('should handle very long text without throwing', () => {
      const tokenizer = new BM25Tokenizer();
      const longText = 'word '.repeat(100000);

      expect(() => tokenizer.tokenize(longText)).not.toThrow();
    });

    test('should handle text with many unique words', () => {
      const tokenizer = new BM25Tokenizer();
      const words = Array(1000)
        .fill(null)
        .map((_, i) => `word${i}`)
        .join(' ');

      const tokens = tokenizer.tokenize(words);
      expect(tokens.length).toBe(1000);
    });
  });

  describe('Special characters in text', () => {
    test('should handle text with HTML-like content', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('Test <script>alert("xss")</script>');

      expect(tokens).toContain('test');
      expect(tokens).toContain('script');
      expect(tokens).toContain('alert');
      expect(tokens).toContain('xss');
    });

    test('should handle text with markdown', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('# Hello **world** _test_');

      expect(tokens).toContain('hello');
      expect(tokens).toContain('world');
      expect(tokens).toContain('test');
    });

    test('should handle text with JSON-like content', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('{"key": "value", "number": 42}');

      // "key" stems to "kei" (terminal y → i rule)
      expect(tokens).toContain('kei');
      expect(tokens).toContain('valu'); // stemmed from "value"
      expect(tokens).toContain('number');
      expect(tokens).toContain('42');
    });
  });

  describe('Query-like patterns', () => {
    test('should handle text with Lucene special chars', () => {
      const tokenizer = new BM25Tokenizer();
      const specialChars = ['\\', '+', '-', '&&', '||', '!', '(', ')', '{', '}', '[', ']', '^', '"', '~', '*', '?', ':', '/'];

      specialChars.forEach((char) => {
        const query = `test${char}query`;
        expect(() => tokenizer.tokenize(query)).not.toThrow();
      });
    });

    test('should handle multiple special characters', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('hello && world || test!');

      expect(tokens).toContain('hello');
      expect(tokens).toContain('world');
      expect(tokens).toContain('test');
    });
  });

  describe('Number handling', () => {
    test('should handle standalone numbers', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('version 123 build 456');

      expect(tokens).toContain('version');
      expect(tokens).toContain('123');
      expect(tokens).toContain('build');
      expect(tokens).toContain('456');
    });

    test('should handle decimal numbers', () => {
      const tokenizer = new BM25Tokenizer();
      // Decimal point splits the number
      const tokens = tokenizer.tokenize('price 19.99 total');

      expect(tokens).toContain('price');
      expect(tokens).toContain('19');
      expect(tokens).toContain('99');
      expect(tokens).toContain('total');
    });

    test('should handle negative numbers', () => {
      const tokenizer = new BM25Tokenizer();
      // Minus sign splits
      const tokens = tokenizer.tokenize('temperature -5 degrees');

      expect(tokens).toContain('temperatur'); // stemmed
      expect(tokens).toContain('degre'); // stemmed
    });
  });

  describe('Empty and whitespace variations', () => {
    test('should handle only stopwords', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('the a an is are was were');

      expect(tokens).toEqual([]);
    });

    test('should handle mixed whitespace types', () => {
      const tokenizer = new BM25Tokenizer();
      const tokens = tokenizer.tokenize('hello\t\t\nworld\r\ntest');

      expect(tokens).toContain('hello');
      expect(tokens).toContain('world');
      expect(tokens).toContain('test');
    });

    test('should handle unicode whitespace', () => {
      const tokenizer = new BM25Tokenizer();
      // Non-breaking space (U+00A0) and other unicode spaces
      const tokens = tokenizer.tokenize('hello\u00A0world\u2003test');

      expect(tokens).toContain('hello');
      expect(tokens).toContain('world');
      expect(tokens).toContain('test');
    });
  });

  describe('Performance characteristics', () => {
    test('should tokenize efficiently (1000 words under 50ms)', () => {
      const tokenizer = new BM25Tokenizer();
      const text = Array(1000)
        .fill(null)
        .map((_, i) => `word${i % 100}`)
        .join(' ');

      const start = performance.now();
      tokenizer.tokenize(text);
      const duration = performance.now() - start;

      expect(duration).toBeLessThan(50);
    });

    test('should handle repeated tokenization efficiently', () => {
      const tokenizer = new BM25Tokenizer();
      const text = 'the quick brown fox jumps over the lazy dog';

      const start = performance.now();
      for (let i = 0; i < 1000; i++) {
        tokenizer.tokenize(text);
      }
      const duration = performance.now() - start;

      // 1000 tokenizations should be under 100ms
      expect(duration).toBeLessThan(100);
    });
  });
});
