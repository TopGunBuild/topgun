import { getPathData } from '../src/client/graph/graph-utils';

const pub1 = 'IkekngFpRcuAmAUjFneY0GGkdGxl4ceFDVgiLWRw39E.NH33KKwMbPNtYKlFhbgHd2vGWESP1Tlq5teRJq3nRUI';
const pub2 = 'M7Xy8-sGHTjWBaf6mGzlhmZyyzgqInTaB81kLHISf5M.ohsXbREqPf1k6GSeN5lgGFEJfK_GJjuRE-YI3WJvoEU';
const graph = {
    '~@john'                                                                                                                                           : {
        '_'                                                                                       : {
            '#': '~@john',
            '>': {
                '~M7Xy8-sGHTjWBaf6mGzlhmZyyzgqInTaB81kLHISf5M.ohsXbREqPf1k6GSeN5lgGFEJfK_GJjuRE-YI3WJvoEU': 1690481207217
            }
        },
        '~M7Xy8-sGHTjWBaf6mGzlhmZyyzgqInTaB81kLHISf5M.ohsXbREqPf1k6GSeN5lgGFEJfK_GJjuRE-YI3WJvoEU': {
            '#': '~M7Xy8-sGHTjWBaf6mGzlhmZyyzgqInTaB81kLHISf5M.ohsXbREqPf1k6GSeN5lgGFEJfK_GJjuRE-YI3WJvoEU'
        }
    },
    '~M7Xy8-sGHTjWBaf6mGzlhmZyyzgqInTaB81kLHISf5M.ohsXbREqPf1k6GSeN5lgGFEJfK_GJjuRE-YI3WJvoEU'                                                         : {
        '_'    : {
            '#': '~M7Xy8-sGHTjWBaf6mGzlhmZyyzgqInTaB81kLHISf5M.ohsXbREqPf1k6GSeN5lgGFEJfK_GJjuRE-YI3WJvoEU',
            '>': { 'alias': 1690481207217, 'auth': 1690481207217, 'epub': 1690481207217, 'pub': 1690481207217 }
        },
        'alias': 'john',
        'auth' : {
            'ek'  : {
                'ct': 'eMGs60f2d2Icyjm5DYpoIBQN+ZJRmQo2Y28yVwN5+7LERUtgeb/J0IedsrZAp7M6DK786vuMQMQ3DZ7eCriRUtaM+fEPxXkch+1ryJmpYkRUvnWDzHxeqgl63T1GiNznnQcpkk/Az/OB9jMamiAycqfrGQZPF86hbBMixg==',
                'iv': '7a4jx2YAiweADtsvsUeQ',
                's' : 'OXV9LsYklSYK'
            }, 's': 'tRgvA73bOd1l72vkyBflnW7IQSUjNf7G960B8AAn3H43A7dIFbFS0Sa5bkrkPXer'
        },
        'epub' : 'z8vPdPNPAL2R6hrTNapvhoJDpbRYQct8EBSk-7sZDJ8.vdmKswphgUXRPQdI-zPYATxbPC1V_UwqD8O0oY3vqjU',
        'pub'  : pub2
    },
    '~M7Xy8-sGHTjWBaf6mGzlhmZyyzgqInTaB81kLHISf5M.ohsXbREqPf1k6GSeN5lgGFEJfK_GJjuRE-YI3WJvoEU/participantRequests/1891985f-76d7-442b-ad5c-602b53595bcb': {
        '_'  : {
            '#': '~M7Xy8-sGHTjWBaf6mGzlhmZyyzgqInTaB81kLHISf5M.ohsXbREqPf1k6GSeN5lgGFEJfK_GJjuRE-YI3WJvoEU/participantRequests/1891985f-76d7-442b-ad5c-602b53595bcb',
            '>': { 'pub': 1690481208434 }
        },
        'pub': pub1
    }
};

describe('GraphUtils', () =>
{
    it('read path 1', () =>
    {
        const keys = [
            '~M7Xy8-sGHTjWBaf6mGzlhmZyyzgqInTaB81kLHISf5M.ohsXbREqPf1k6GSeN5lgGFEJfK_GJjuRE-YI3WJvoEU',
            'participantRequests',
            '1891985f-76d7-442b-ad5c-602b53595bcb',
        ];
        const result = getPathData(keys, graph);

        expect(result.value['pub'] === pub1);
    });

    it('read path 2', () =>
    {
        const keys = [
            '~M7Xy8-sGHTjWBaf6mGzlhmZyyzgqInTaB81kLHISf5M.ohsXbREqPf1k6GSeN5lgGFEJfK_GJjuRE-YI3WJvoEU/participantRequests/1891985f-76d7-442b-ad5c-602b53595bcb'
        ];
        const result = getPathData(keys, graph);

        expect(result.value['pub'] === pub1);
    });

    it('read path 3', () =>
    {
        const keys = [
            '~M7Xy8-sGHTjWBaf6mGzlhmZyyzgqInTaB81kLHISf5M.ohsXbREqPf1k6GSeN5lgGFEJfK_GJjuRE-YI3WJvoEU',
            'participantRequests',
            '1891985f-76d7-442b-ad5c-602b53595bcb',
            'pub'
        ];
        const result = getPathData(keys, graph);

        expect(result.value === pub1);
    });

    it('read path 4', () =>
    {
        const keys = [
            '~@john',
            '~M7Xy8-sGHTjWBaf6mGzlhmZyyzgqInTaB81kLHISf5M.ohsXbREqPf1k6GSeN5lgGFEJfK_GJjuRE-YI3WJvoEU'
        ];
        const result = getPathData(keys, graph);

        expect(result.value['pub'] === pub2);
    });
});

