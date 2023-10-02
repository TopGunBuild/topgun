import { TGServer } from '../src/server';
import { TGClient } from '../src/client';

let server: TGServer, client: TGClient;

const countries = [

];

describe('Range query', () =>
{
    beforeEach(async () =>
    {
        client  = new TGClient({
            peers: [{
                hostname: '127.0.0.1',
                port    : 5000,
            }]
        });

        server = new TGServer({
            port      : 5000,
            log       : {
                enabled: true,
            }
        });
    });

    afterEach(async () =>
    {
        await Promise.all([
            client.disconnect(),
            server.close(),
        ]);
    });

    it('range query', async () =>
    {

    });
});