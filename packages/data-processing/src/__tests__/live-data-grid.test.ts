import { wait } from "@topgunbuild/test-utils";
import { createTestData } from "./test-database";


describe('LiveDataGrid tests with page size 10 and page offset 10', () => {
    const testData = createTestData(10, 10);

    it('should be able to query data', async () => {
        await testData.grid.fetchFromDatabase(true);
        expect(testData.result).toEqual([11, 12, 13, 14, 15, 16, 17, 18, 19, 20]);
    });

    it('insert row with id 1.1', async () => {
        testData.db.insert(testData.db.generateById(1.1));
        await wait(10);
        expect(testData.result).toEqual([10, 11, 12, 13, 14, 15, 16, 17, 18, 19]);
    });

    it('insert row with id 0.1', async () => {
        testData.db.insert(testData.db.generateById(0.1));
        await wait(10);
        expect(testData.result).toEqual([9, 10, 11, 12, 13, 14, 15, 16, 17, 18]);
    });

    it('delete row with id 0.1', async () => {
        testData.db.deleteById(0.1);
        await wait(10);
        expect(testData.result).toEqual([10, 11, 12, 13, 14, 15, 16, 17, 18, 19]);
    });

    it('delete row with id 5', async () => {
        testData.db.deleteById(5);
        await wait(10);
        expect(testData.result).toEqual([11, 12, 13, 14, 15, 16, 17, 18, 19, 20]);
    });

    it('insert row with id 12.1', async () => {
        testData.db.insert(testData.db.generateById(12.1));
        await wait(10);
        expect(testData.result).toEqual([11, 12, 12.1, 13, 14, 15, 16, 17, 18, 19]);
    });

    it('insert row with id 10.1', async () => {
        testData.db.insert(testData.db.generateById(10.1));
        await wait(10);
        expect(testData.result).toEqual([10.1, 11, 12, 12.1, 13, 14, 15, 16, 17, 18]);
    });

    it('delete row with id 15', async () => {
        testData.db.deleteById(15);
        await wait(10);
        expect(testData.result).toEqual([10.1, 11, 12, 12.1, 13, 14, 16, 17, 18, 19]);
    });

    it('delete row with id 10.1', async () => {
        testData.db.deleteById(10.1);
        await wait(10);
        expect(testData.result).toEqual([11, 12, 12.1, 13, 14, 16, 17, 18, 19, 20]);
    });

    it('delete row with id 12.1', async () => {
        testData.db.deleteById(12.1);
        await wait(10);
        expect(testData.result).toEqual([11, 12, 13, 14, 16, 17, 18, 19, 20, 21]);
    });

    it('insert row with id 15', async () => {
        testData.db.insert(testData.db.generateById(15));
        await wait(10);
        expect(testData.result).toEqual([11, 12, 13, 14, 15, 16, 17, 18, 19, 20]);
    });

    const ids = [11, 12, 13, 14, 15, 16, 17, 18, 19, 20];

    it('insert row with id 22.1', async () => {
        testData.db.insert(testData.db.generateById(22.1));
        await wait(10);
        expect(testData.result).toEqual(ids);
    });

    it('insert row with id 20.1', async () => {
        testData.db.insert(testData.db.generateById(20.1));
        await wait(10);
        expect(testData.result).toEqual(ids);
    });

    it('delete row with id 25', async () => {
        testData.db.deleteById(25);
        await wait(10);
        expect(testData.result).toEqual(ids);
    });

    it('delete row with id 20.1', async () => {
        testData.db.deleteById(20.1);
        await wait(10);
        expect(testData.result).toEqual(ids);
    });

    it('delete row with id 22.1', async () => {
        testData.db.deleteById(22.1);
        await wait(10);
        expect(testData.result).toEqual(ids);
    });

    it('insert row with id 25', async () => {
        testData.db.insert(testData.db.generateById(25));
        await wait(10);
        expect(testData.result).toEqual(ids);
    });

    it('remove rows with id 21-30', async () => {
        testData.db.deleteById(21);
        testData.db.deleteById(22);
        testData.db.deleteById(23);
        testData.db.deleteById(24);
        testData.db.deleteById(25);
        testData.db.deleteById(26);
        testData.db.deleteById(27);
        testData.db.deleteById(28);
        testData.db.deleteById(29);
        testData.db.deleteById(30);
        await wait(10);
        expect(testData.result).toEqual(ids);
    });

    it('delete row with id 1', async () => {
        testData.db.deleteById(1);
        await wait(10);
        expect(testData.result).toEqual([12, 13, 14, 15, 16, 17, 18, 19, 20, 31]);
    });

    it('remove rows with id 31-41', async () => {
        testData.db.deleteById(31);
        testData.db.deleteById(32);
        testData.db.deleteById(33);
        testData.db.deleteById(34);
        testData.db.deleteById(35);
        testData.db.deleteById(36);
        testData.db.deleteById(37);
        testData.db.deleteById(38);
        testData.db.deleteById(39);
        testData.db.deleteById(40);
        testData.db.deleteById(41);
        await wait(10);
        expect(testData.result).toEqual([12, 13, 14, 15, 16, 17, 18, 19, 20, 42]);
    });

    it('delete row with id 42', async () => {
        testData.db.deleteById(42);
        await wait(10);
        expect(testData.result).toEqual([12, 13, 14, 15, 16, 17, 18, 19, 20, 43]);
    });
}); 

describe('LiveDataGrid tests with page size 10 and page offset 0', () => {
    const testData = createTestData(0, 10);

    it('should be able to query data', async () => {
        await testData.grid.fetchFromDatabase(true);
        expect(testData.result).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    });

    it('insert row with id 1.1', async () => {
        testData.db.insert(testData.db.generateById(1.1));
        await wait(10);
        expect(testData.result).toEqual([1, 1.1, 2, 3, 4, 5, 6, 7, 8, 9]);
    });

    it('insert row with id 0.1', async () => {
        testData.db.insert(testData.db.generateById(0.1));
        await wait(10);
        expect(testData.result).toEqual([0.1, 1, 1.1, 2, 3, 4, 5, 6, 7, 8]);
    });

    it('delete row with id 0.1', async () => {
        testData.db.deleteById(0.1);
        await wait(10);
        expect(testData.result).toEqual([1, 1.1, 2, 3, 4, 5, 6, 7, 8, 9]);
    });

    const ids = [1, 1.1, 2, 3, 4, 6, 7, 8, 9, 10];

    it('delete row with id 5', async () => {
        testData.db.deleteById(5);
        await wait(10);
        expect(testData.result).toEqual(ids);
    });

    it('insert row with id 12.1', async () => {
        testData.db.insert(testData.db.generateById(12.1));
        await wait(10);
        expect(testData.result).toEqual(ids);
    });

    it('insert row with id 10.1', async () => {
        testData.db.insert(testData.db.generateById(10.1));
        await wait(10);
        expect(testData.result).toEqual(ids);
    });
}); 

describe('LiveDataGrid tests with page size 10 and page offset 90', () => {
    const testData = createTestData(90, 10);

    it('should be able to query data', async () => {
        await testData.grid.fetchFromDatabase(true);
        expect(testData.result).toEqual([91, 92, 93, 94, 95, 96, 97, 98, 99, 100]);
    });

    it('insert row with id 1.1', async () => {
        testData.db.insert(testData.db.generateById(1.1));
        await wait(10);
        expect(testData.result).toEqual([90, 91, 92, 93, 94, 95, 96, 97, 98, 99]);
    });

    it('insert row with id 0.1', async () => {
        testData.db.insert(testData.db.generateById(0.1));
        await wait(10);
        expect(testData.result).toEqual([89, 90, 91, 92, 93, 94, 95, 96, 97, 98]);
    });

    it('delete row with id 0.1', async () => {
        testData.db.deleteById(0.1);
        await wait(10);
        expect(testData.result).toEqual([90, 91, 92, 93, 94, 95, 96, 97, 98, 99]);
    });

    it('delete row with id 5', async () => {
        testData.db.deleteById(5);
        await wait(10);
        expect(testData.result).toEqual([91, 92, 93, 94, 95, 96, 97, 98, 99, 100]);
    });

    it('insert row with id 92.1', async () => {
        testData.db.insert(testData.db.generateById(92.1));
        await wait(10);
        expect(testData.result).toEqual([91, 92, 92.1, 93, 94, 95, 96, 97, 98, 99]);
    });

    it('insert row with id 90.1', async () => {
        testData.db.insert(testData.db.generateById(90.1));
        await wait(10);
        expect(testData.result).toEqual([90.1, 91, 92, 92.1, 93, 94, 95, 96, 97, 98]);
    });

    it('delete row with id 95', async () => {
        testData.db.deleteById(95);
        await wait(10);
        expect(testData.result).toEqual([90.1, 91, 92, 92.1, 93, 94, 96, 97, 98, 99]);
    });

    it('delete row with id 90.1', async () => {
        testData.db.deleteById(90.1);
        await wait(10);
        expect(testData.result).toEqual([91, 92, 92.1, 93, 94, 96, 97, 98, 99, 100]);
    });

    it('delete row with id 92.1', async () => {
        testData.db.deleteById(92.1);
        await wait(10);
        expect(testData.result).toEqual([91, 92, 93, 94, 96, 97, 98, 99, 100]);
    });

    it('insert row with id 95', async () => {
        testData.db.insert(testData.db.generateById(95));
        await wait(10);
        expect(testData.result).toEqual([91, 92, 93, 94, 95, 96, 97, 98, 99, 100]);
    });
}); 

describe('LiveDataGrid tests with page size 10 and page offset 100', () => {
    const testData = createTestData(100, 10);

    it('should be able to query data', async () => {
        await testData.grid.fetchFromDatabase(true);
        expect(testData.result).toEqual([]);
    });

    it('insert row with id 1.1', async () => {
        testData.db.insert(testData.db.generateById(1.1));
        await wait(10);
        expect(testData.result).toEqual([100]);
    });

    it('insert row with id 0.1', async () => {
        testData.db.insert(testData.db.generateById(0.1));
        await wait(10);
        expect(testData.result).toEqual([99, 100]);
    });

    it('delete row with id 0.1', async () => {
        testData.db.deleteById(0.1);
        await wait(10);
        expect(testData.result).toEqual([100]);
    });

    it('delete row with id 5', async () => {
        testData.db.deleteById(5);
        await wait(10);
        expect(testData.result).toEqual([]);
    });
}); 