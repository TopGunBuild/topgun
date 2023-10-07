import { TGServer } from '../src/server';
import { TGClient } from '../src/client';
import { isEven, isOdd, wait } from './test-util';

let server: TGServer, client1: TGClient, client2: TGClient;

export const countries = [
    { name: 'Albania', code: 'AL' },
    { name: 'Algeria', code: 'DZ' },
    { name: 'AndorrA', code: 'AD' },
    { name: 'Angola', code: 'AO' },
    { name: 'Anguilla', code: 'AI' },
    { name: 'Austria', code: 'AT' },
    { name: 'Azerbaijan', code: 'AZ' },
    { name: 'Bahamas', code: 'BS' },
    { name: 'Barbados', code: 'BB' },
    { name: 'Brazil', code: 'BR' },
    { name: 'Cameroon', code: 'CM' },
    { name: 'Canada', code: 'CA' },
    { name: 'Colombia', code: 'CO' },
    { name: 'Cuba', code: 'CU' },
    { name: 'Denmark', code: 'DK' },
    { name: 'Dominica', code: 'DM' },
    { name: 'Egypt', code: 'EG' },
];

const putCountries = async (_client: TGClient, cond: (v: number) => boolean) =>
{
    let index = 0;

    for (const c of countries)
    {
        if (cond(index))
        {
            await _client.get('countries').get(c.name).put(c);
        }
        index++;
    }
};

describe('Range query', () =>
{
    beforeEach(async () =>
    {
        client1 = new TGClient({
            peers: [{
                hostname: '127.0.0.1',
                port    : 5000,
            }]
        });
        client2 = new TGClient({
            peers: [{
                hostname: '127.0.0.1',
                port    : 5000,
            }]
        });
        server  = new TGServer({
            port: 5000,
        });

        await Promise.all([
            server.waitForReady(),
            client1.waitForConnect(),
            client2.waitForConnect()
        ]);
    });

    afterEach(async () =>
    {
        await Promise.all([
            client1.disconnect(),
            client2.disconnect(),
            server.close(),
        ]);
    });

    it('prefix/limit', async () =>
    {
        await putCountries(client1, isOdd);
        await wait(40);

        const receivedPackets = [];
        const stream          = client1
            .get('countries')
            .prefix('A')
            .limit(2)
            .on<{name: string, code: string}>();

        (async () =>
        {
            for await (const { key } of stream)
            {
                receivedPackets.push(key);
            }
        })();

        await wait(40);
        expect(receivedPackets.length).toBe(2);

        await putCountries(client2, isEven);
        await wait(40);

        // console.log(receivedPackets);
        expect(receivedPackets.length).toBe(6);
        stream.destroy();
    });
});