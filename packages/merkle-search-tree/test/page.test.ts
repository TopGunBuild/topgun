import { Digest, PageDigest, ValueDigest } from '../src';
import { Page, Node, splitOffLt } from '../src';
import { assertTree } from '../src/assert-tree';

const MOCK_VALUE: ValueDigest<1>    = new ValueDigest(new Digest(new Uint8Array(1).fill(0)));
const MOCK_PAGE_HASH: PageDigest = new PageDigest(new Uint8Array(16).fill(0));

describe('Page Split Tests', () =>
{
  test('test_split_page_empty', () =>
  {
    let gtePage: Page<1, any>|null = new Page(42, []);
    const cb = (_: any) => {};
    expect(() => splitOffLt(gtePage, 5, cb)).toThrowError('No nodes in this page.');
  });

  test('test_split_page_single_node_lt', () =>
  {
    let gtePage: Page<1, number> = new Page(42, [new Node(2, MOCK_VALUE, null)]);
    gtePage.treeHash       = MOCK_PAGE_HASH;

    const ltPage = splitOffLt(gtePage, 5, updatedPaged =>
    {
      gtePage = updatedPaged;
    });
    expect(gtePage).toBeNull();

    expect(ltPage.level).toBe(42);
    expect(ltPage.treeHash).toBe(MOCK_PAGE_HASH);
    expect(ltPage.nodes).toEqual([new Node(2, MOCK_VALUE, null)]);
  });

  test('test_split_page_single_node_gt', () =>
  {
    let gtePage: Page<1, number> = new Page(42, [new Node(2, MOCK_VALUE, null)]);
    gtePage.treeHash       = MOCK_PAGE_HASH;

    const ltPage = splitOffLt(gtePage, 1, updatedPaged =>
    {
      gtePage = updatedPaged;
    });

    expect(ltPage).toBeNull();

    expect(gtePage.level).toBe(42);
    expect(gtePage.treeHash).toBe(MOCK_PAGE_HASH);
    expect(gtePage.nodes).toEqual([new Node(2, MOCK_VALUE, null)]);
  });

  test('test_split_page_single_node_gt_with_high_page_split', () =>
  {
    let highPage      = new Page(40, [
      new Node(10, MOCK_VALUE, null),
      new Node(15, MOCK_VALUE, null),
    ]);
    highPage.treeHash = MOCK_PAGE_HASH;

    let page      = new Page<any, number>(42, [new Node(5, MOCK_VALUE, null)]);
    page.treeHash = MOCK_PAGE_HASH;
    page.insertHighPage(highPage);

    let ltPage = splitOffLt(page, 12, updatedPaged =>
    {
      page = updatedPaged;
    });
    // console.log({
    //   page,
    //   ltPage
    // })
    expect(page.level).toBe(40);
    expect(page.treeHash).toBeNull();
    expect(page.nodes).toEqual([new Node(15, MOCK_VALUE, null)]);
    expect(page.highPage).toBeNull();

    expect(ltPage.level).toBe(42);
    expect(ltPage.treeHash).toBeNull();
    expect(ltPage.nodes).toEqual([new Node(5, MOCK_VALUE, null)]);
    expect(ltPage.highPage.nodes).toEqual([new Node(10, MOCK_VALUE, null)]);
    expect(ltPage.highPage.treeHash).toBeNull();
  });

  test('test_split_page_single_node_gt_with_child_page_split', () =>
  {
    const child2 = new Page(40, [
      new Node(1, MOCK_VALUE, null),
      new Node(3, MOCK_VALUE, null),
    ]);
    const child1 = new Page(41, [new Node(4, MOCK_VALUE, child2)]);

    let page      = new Page(42, [new Node(5, MOCK_VALUE, child1)]);
    page.treeHash = MOCK_PAGE_HASH;

    const ltPage = splitOffLt(page, 2, updatedPage =>
    {
      page = updatedPage;
    });

    expect(page.level).toBe(42);
    expect(page.treeHash).toBeNull();
    expect(page.nodes).toEqual([
      new Node(
        5,
        MOCK_VALUE,
        new Page(41, [new Node(4, MOCK_VALUE, new Page(40, [new Node(3, MOCK_VALUE, null)]))]))
    ]);

    expect(ltPage.level).toBe(40);
    expect(ltPage.treeHash).toBeNull();
  });

  test('test_split_page_eq', () =>
  {
    let gtePage: Page<any, number> = new Page(42, [
      new Node(1, MOCK_VALUE, null),
      new Node(2, MOCK_VALUE, null),
      new Node(4, MOCK_VALUE, null),
    ]);
    gtePage.treeHash       = MOCK_PAGE_HASH;

    const ltPage = splitOffLt(gtePage, 2, updatedPage =>
    {
      gtePage = updatedPage;
    });
    expect(gtePage.level).toBe(42);
    expect(gtePage.treeHash).toBeNull();
    expect(gtePage.nodes).toEqual([
      new Node(2, MOCK_VALUE, null),
      new Node(4, MOCK_VALUE, null)
    ]);

    expect(ltPage.level).toBe(42);
    expect(ltPage.treeHash).toBeNull();
    expect(ltPage.nodes).toEqual([new Node(1, MOCK_VALUE, null)]);
  });

  test('test_split_page_lt', () =>
  {
    let gtePage: Page<any, number> = new Page(42, [
      new Node(1, MOCK_VALUE, null),
      new Node(2, MOCK_VALUE, null),
      new Node(4, MOCK_VALUE, null),
    ]);
    gtePage.treeHash       = MOCK_PAGE_HASH;

    const ltPage = splitOffLt(gtePage, 3, updatedPage =>
    {
      gtePage = updatedPage;
    });
    expect(gtePage.level).toBe(42);
    expect(gtePage.treeHash).toBeNull();
    expect(gtePage.nodes).toEqual([new Node(4, MOCK_VALUE, null)]);

    expect(ltPage.level).toBe(42);
    expect(ltPage.treeHash).toBeNull();
    expect(ltPage.nodes).toEqual([
      new Node(1, MOCK_VALUE, null),
      new Node(2, MOCK_VALUE, null),
    ]);
  });

  test('test_split_page_all_gt', () =>
  {
    let gtePage: Page<any, number> = new Page(42, [
      new Node(1, MOCK_VALUE, null),
      new Node(2, MOCK_VALUE, null),
      new Node(4, MOCK_VALUE, null),
    ]);
    gtePage.treeHash       = MOCK_PAGE_HASH;

    const ltPage = splitOffLt(gtePage, 0, updatedPage =>
    {
      gtePage = updatedPage;
    });
    expect(gtePage.level).toBe(42);
    expect(gtePage.treeHash).toBe(MOCK_PAGE_HASH);
    expect(gtePage.nodes).toEqual([
      new Node(1, MOCK_VALUE, null),
      new Node(2, MOCK_VALUE, null),
      new Node(4, MOCK_VALUE, null),
    ]);

    expect(ltPage).toBeNull();
  });

  test('test_split_page_all_lt', () =>
  {
    let gtePage: Page<any, number> = new Page(42, [
      new Node(1, MOCK_VALUE, null),
      new Node(2, MOCK_VALUE, null),
      new Node(4, MOCK_VALUE, null),
    ]);
    gtePage.treeHash       = MOCK_PAGE_HASH;

    const ltPage = splitOffLt(gtePage, 10, updatedPage =>
    {
      gtePage = updatedPage;
    });
    expect(gtePage).toBeNull();

    expect(ltPage.level).toBe(42);
    expect(ltPage.treeHash).toBe(MOCK_PAGE_HASH);
    expect(ltPage.nodes).toEqual([
      new Node(1, MOCK_VALUE, null),
      new Node(2, MOCK_VALUE, null),
      new Node(4, MOCK_VALUE, null),
    ]);
  });

  test('test_upsert_less_than_split_child', () =>
  {
    let p = new Page(1, [new Node(4, MOCK_VALUE, null)]);
    p.upsert(3, 0, MOCK_VALUE);
    p.upsert(1, 0, MOCK_VALUE);
    p.upsert(2, 1, MOCK_VALUE);

    assertTree(p);
  });

  test('test_split_page_recursive_lt_pointer', () =>
  {
    let ltPointerPage      = new Page(52, [new Node(86, MOCK_VALUE, null)]);
    ltPointerPage.treeHash = MOCK_PAGE_HASH;

    let root      = new Page(42, [new Node(161, MOCK_VALUE, ltPointerPage)]);
    root.treeHash = MOCK_PAGE_HASH;

    const key = 160;

    const ltPage = splitOffLt(root, key, updatedPage =>
    {
      root = updatedPage;
    });
    expect(ltPage.level).toBe(52);
    expect(ltPage.nodes[0].key).toBe(86);
  });

  test('test_split_page_recursive_high_page', () =>
  {
    let highPage      = new Page(32, [new Node(44, MOCK_VALUE, null)]);
    highPage.treeHash = MOCK_PAGE_HASH;

    let root      = new Page(42, [new Node(42, MOCK_VALUE, null)]);
    root.treeHash = MOCK_PAGE_HASH;
    root.insertHighPage(highPage);

    const key = 43;

    const ltPage = splitOffLt(root, key, updatedPage =>
    {
      root = updatedPage;
    });
    expect(ltPage.level).toBe(42);
    expect(ltPage.nodes[0].key).toBe(42);

    expect(root.level).toBe(32);
    expect(root.nodes[0].key).toBe(44);
  });
});

