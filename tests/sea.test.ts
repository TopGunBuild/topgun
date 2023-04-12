import * as SEA from '../src/sea';
import { TGClient } from '../src/client';

describe('SEA', () =>
{
    const client = new TGClient();

    it('encrypt/decrypt', async () =>
    {
        const pair       = await SEA.pair();
        const data       = 'foo';
        const ciphertext = (await SEA.encrypt(data, pair.epriv)) as string;
        expect(await SEA.decrypt(ciphertext, pair.epriv)).toBe(data);
    });

    it('sign', async () =>
    {
        const pair      = await SEA.pair();
        const otherPair = await SEA.pair();
        const data      = 'foo';
        const signed    = await SEA.sign(data, pair);
        expect(await SEA.verify(signed, pair.pub)).toBe(true);
        expect(await SEA.verify(signed, otherPair.pub)).toBe(false);
    });

    it('quickstart', async () =>
    {
        const pair = await SEA.pair();
        const enc  = await SEA.encrypt('hello self', pair);
        const data = await SEA.sign(enc, pair);
        const msg  = await SEA.verify(data, pair.pub);
        if (msg)
        {
            const dec = (await SEA.decrypt(msg, pair)) as string;
            expect(dec).toBe('hello self');

            const proof = await SEA.work(dec, pair);
            const check = await SEA.work('hello self', pair);
            expect(proof).toBe(check);

            const alice = await SEA.pair();
            const bob   = await SEA.pair();
            const aes   = await SEA.secret(bob.epub, alice);
            const enc1  = await SEA.encrypt('shared data', aes as string);
            const aes1  = await SEA.secret(alice.epub, bob);
            const dec1  = await SEA.decrypt(enc1, aes1 as string);
            expect(dec1).toBe('shared data');
        }
    });

    it('quickwrong', async () =>
    {
        const alice = await SEA.pair();
        const bob   = await SEA.pair();

        const data      = await SEA.sign('asdf', alice);
        const isVerify1 = await SEA.verify(data, bob.pub);
        expect(isVerify1).toBe(false);

        const isVerify2 = await SEA.verify(data.slice(0, 20) + data.slice(21), alice.pub);
        expect(isVerify2).toBe(false);

        const enc1 = await SEA.encrypt('secret', alice);
        const dec2 = await SEA.decrypt(enc1, bob);
        expect(dec2).toBe(undefined);
    });

    it('double sign', async () =>
    {
        const pair = await SEA.pair();
        const sig1 = await SEA.sign('hello world', pair);
        const dup1 = await SEA.sign(sig1, pair);
        expect(dup1).toBe(sig1);
    });

    it('register/auth', async () =>
    {
        const user    = client.user();
        const newUser = await user.create('carl', 'testing123');
        user.leave();
        const authUser = await user.auth('carl', 'testing123');
        expect(newUser.pub).toBe(authUser?.pub);
    });

    it('save & read encrypt', async () =>
    {
        const pair = await SEA.pair();
        const data = await SEA.encrypt('hi', pair.epriv);
        const is   = data.slice();

        client.get('a').get('d').put(data, ack =>
        {
            expect(ack.err).toBeFalsy();
            setTimeout(() =>
            {
                client.get('a').get('d').once(data =>
                {
                    expect(data).toBe(is);
                });
            })
        });
    });

    it('set user ref should be found', async () =>
    {
        const user = client.user();
        const msg  = { what: 'hello world' };
        await user.create('zach', 'password');

        const ref = user.get('who')?.get('all').set(msg);
        console.log(ref);
        user.get('who')?.get('said').set(ref);
        user.get('who')?.get('said').map().once(data =>
        {
            console.log('*****', data);
            // expect(data.what).to.be.ok();
        })
    });
});
