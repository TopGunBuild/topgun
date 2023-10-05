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
        server = new TGServer({
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

    it('range query', async () =>
    {
        const putCountries = async (cond: (v: number) => boolean) =>
        {
            let index = 0;

            for (const c of countries)
            {
                if (cond(index))
                {
                    await client2
                        .get('countries')
                        .get(c.name)
                        .put(c);
                }
                index++;
            }
        };

        await putCountries(isOdd);

        await wait(40);

        const stream = client1
            .get('countries')
            .prefix('A')
            .on<{name: string, code: string}>();

        const receivedPackets = [];

        (async () =>
        {
            for await (const { value, key } of stream)
            {
                receivedPackets.push(key);
            }
        })();

        await putCountries(isEven);

        await wait(40);

        expect(receivedPackets.length === 7).toBeTruthy();
        stream.destroy();
    });
});