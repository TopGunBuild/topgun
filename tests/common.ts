import { TGClient } from '../src/client';
import { TGServer } from '../src/server';

const port = 3457;
let server: TGServer, client: TGClient;

describe('Common', () =>
{
    beforeEach(() =>
    {
        client = new TGClient();
        server = new TGServer({
            port: 8765
        });
    });
    afterEach(async () =>
    {
        await client.disconnect();
    });
});