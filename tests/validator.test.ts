import { expectErr, expectOk } from './test-util';
import { validator } from '../src/validator';

const getTopGunData = () => ({
    user: {
        _: {
            '#': 'user',
            '>': {
                name: 1682701808609,
                email: 1682701808609,
                said: 1682701831454,
            },
        },
        name: 'Mark',
        email: 'mark@minigun.tech',
        said: { '#': 'user/said' },
    },
});

describe('Graph format', () => {
    it('Valid graph data', () => {
        const graph = getTopGunData();
        const result = validator(graph);

        expectOk(result, graph);
    });

    it('Graph nullable', () => {
        const result = validator(null);
        expectErr(result, `Root graph must be object`);
    });

    it('Graph undefined', () => {
        const result = validator(undefined);
        expectErr(result, `Root graph must be object`);
    });

    it('Graph string', () => {
        const result = validator('');
        expectErr(result, `Root graph must be object`);
    });

    it('Graph number', () => {
        const result = validator(123);
        expectErr(result, `Root graph must be object`);
    });

    it('Graph array', () => {
        const result = validator([]);
        expectErr(result, `Root graph must be object`);
    });

    it('Soul not present in graph', () => {
        const graph = getTopGunData();
        graph.user._['#'] = 'xxx'; // Change soul

        const result = validator(graph);
        expectErr(result, `Soul must be present in root graph`);
    });

    it('Soul must be present in graph', () => {
        const graph = getTopGunData();
        graph.user._['#'] = 'xxx'; // Change soul

        const result = validator(graph);
        expectErr(result, `Soul must be present in root graph`);
    });

    it('Soul must be string', () => {
        const graph = getTopGunData();
        graph.user._['#'] = null; // Change soul

        const result = validator(graph);
        expectErr(result, `Soul must be string in _.['#']`);
    });

    it('Node state must be object', () => {
        const graph = getTopGunData();
        graph.user._['>'] = null; // Change state

        expectErr(
            validator(graph),
            `Node state must be object in path _.['>']`,
        );
    });
});
