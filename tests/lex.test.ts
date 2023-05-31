import { TGClient, TGOptionsGet } from '../src/client';
import { createServer } from '../src/server';

function get(
    nodeSoul: string,
    opts?: TGOptionsGet
)
{

}

const port   = 3457;
const client = new TGClient({
    peers         : [`http://127.0.0.1:${port}`],
    persistSession: false
});
const server = createServer({ port });

describe('LEX', () =>
{
    it('', async () =>
    {

    });
});
