import { decrypt } from './decrypt';
import { work } from './work';
import { Client } from '../client/client';
import { Pair } from './pair';
import { isObject } from '../utils/is-object';
import { isString } from '../utils/is-string';
import { isNumber } from '../utils/is-number';

export type AuthOptions = {
    timeout?: number
};
export type AuthResult = {
    readonly alias: string
    readonly epriv: string
    readonly epub: string
    readonly priv: string
    readonly pub: string
};

const DEFAULT_OPTS = {
    timeout: 1000
};

export async function authenticateAccount(
    ident: any,
    password: string,
    encoding: 'base64' | 'utf8' | 'hex' = 'base64'
): Promise<undefined
    |{
    readonly alias: string
    readonly epriv: string
    readonly epub: string
    readonly priv: string
    readonly pub: string
}>
{
    if (!ident || !ident.auth)
    {
        return undefined;
    }

    let decrypted: any;
    try
    {
        const proof = await work(password, ident.auth.s, { encode: encoding });

        decrypted = await decrypt(ident.auth.ek, proof, {
            encode: encoding
        });
    }
    catch (err)
    {
        const proof = await work(password, ident.auth.s, { encode: 'utf8' });
        decrypted   = await decrypt(ident.auth.ek, proof, {
            encode: encoding
        });
    }

    if (!decrypted)
    {
        return undefined;
    }

    return {
        alias: ident.alias as string,
        epriv: decrypted.epriv as string,
        epub : ident.epub as string,
        priv : decrypted.priv as string,
        pub  : ident.pub as string
    }
}

export async function authenticateIdentity(
    client: Client,
    soul: string,
    password: string,
    encoding: 'base64' | 'utf8' | 'hex' = 'base64'
): Promise<|undefined
    |{
    readonly alias: string
    readonly epriv: string
    readonly epub: string
    readonly priv: string
    readonly pub: string
}>
{
    const ident = await client.get(soul).then();
    return authenticateAccount(ident, password, encoding);
}

export function authenticate(
    client: Client,
    pair: Pair,
    opt?: AuthOptions
): Promise<AuthResult>
export function authenticate(
    client: Client,
    alias: string,
    password: string,
    opt?: AuthOptions
): Promise<AuthResult>
export async function authenticate(
    client: Client,
    aliasOrPair: string|Pair,
    passwordOrOpt: string|AuthOptions,
    maybeOptions?: AuthOptions
): Promise<AuthResult>
{
    let pair: Pair, alias: string, password: string, options: AuthOptions;

    if (isObject(aliasOrPair) && (aliasOrPair.pub || aliasOrPair.epub))
    {
        pair = aliasOrPair as Pair;
    }
    else if (isString(aliasOrPair))
    {
        alias = aliasOrPair;
    }

    if (isString(passwordOrOpt))
    {
        password = passwordOrOpt;
    }

    if (isObject(passwordOrOpt))
    {
        options = passwordOrOpt;
    }
    else if (isObject(maybeOptions))
    {
        options = maybeOptions;
    }
    else
    {
        options = DEFAULT_OPTS;
    }

    // Auth by alias and password
    if (alias)
    {
        const aliasSoul = `~@${alias}`;

        if (!isObject(options))
        {
            options = {};
        }
        if (!isNumber(options.timeout))
        {
            options.timeout = DEFAULT_OPTS.timeout;
        }

        let idents = client.graph.connectorCount() === 0
            ? await client.get(aliasSoul).promise({ timeout: options.timeout }).then()
            : await client.get(aliasSoul).then();

        if (!isObject(idents))
        {
            idents = {};
        }

        for (const soul in idents)
        {
            if (soul === '_')
            {
                continue;
            }

            let pair;

            try
            {
                pair = await authenticateIdentity(client, soul, password);
            }
            catch (e: any)
            {
                console.warn(e.stack || e);
            }

            if (pair)
            {
                return pair
            }
        }

        throw new Error('Wrong alias or password.');
    }

    // Auth by pair
    if (pair)
    {
        return {
            ...pair,
            alias: '~' + pair.pub
        }
    }

    throw new Error('There is no pair or alias and password.');
}