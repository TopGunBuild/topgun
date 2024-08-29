import { LevelKey, MockHasher } from './test-util';
import { Digest } from '../src';

describe('testMockHasher', () =>
{
  const hasher = new MockHasher();

  it('Test case', () =>
  {
    let got = hasher.hash(new LevelKey('A', 0));
    expect(Digest.level(got) === 0).toBeTruthy();

    got = hasher.hash(new LevelKey('A', 1));
    expect(Digest.level(got) === 1).toBeTruthy();

    got = hasher.hash(new LevelKey('key_A', 2));
    expect(Digest.level(got) === 2).toBeTruthy();

    got = hasher.hash(new LevelKey('key_A', 10));
    expect(Digest.level(got) === 10).toBeTruthy();
  });
});
