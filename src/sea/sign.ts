import Buffer from 'topgun-buffer';
import { check, ecdsa, jwk, parse } from './settings';
import { sha256 } from './sha256';
import { crypto } from './shims';
import { pubFromSoul } from './soul';
import { verify, VerifyData } from './verify';
import { TGGraphData, TGNode, TGOptionsPut, TGValue } from '../types';
import { TGClient, TGLink } from '../client';
import { isString, isObject, isUndefined } from 'topgun-typed';
import { Policy } from './policy';
import { PairBase } from './pair';

const DEFAULT_OPTS: {
    readonly encode?: string;
    readonly raw?: boolean;
    readonly check?: {
        readonly m: any;
        readonly s: string;
    };
} = {
    encode: 'base64',
};

export async function verifyCertificate(
    client: TGClient,
    cert: { readonly m: string; readonly s: string },
    userPub: string,
    soulPub: string,
    fullPath: string[],
): Promise<false | VerifyData> 
{
    if (cert && cert.m && cert.s && userPub && soulPub) 
    {
        // check if "pub" (of the graph owner) really issued this cert
        const data = await verify(cert, soulPub);
        const putDate = new Date().getTime();
        const certDate = (data && data.e) || 0;

        if (putDate > certDate) 
        {
            console.warn('Certificate expired.');
            return false;
        }

        // "data.c" = a list of certified users
        const certified = data && data.c;
        // "data.w" = lex WRITE permission, in the future, there will be "data.r" which means lex READ permission
        const writePermission = data && data.w;

        if (
            certified &&
            writePermission &&
            (certified === '*' || certified.includes(userPub))
        ) 
        {
            // ok, now "certifying" is in the "certifyings" list, but is "path" allowed? Check path
            const key = fullPath.pop() as string;
            const path = fullPath.join('/');

            const writePermissions = Array.isArray(writePermission)
                ? writePermission
                : isObject(writePermission) || isString(writePermission)
                    ? [writePermission]
                    : [];

            for (const lex of writePermissions) 
            {
                const policy = new Policy(lex, path, key, userPub);

                if (policy.match()) 
                {
                    // Is Certificant forced to present in Path
                    if (policy.hasCertificatePathError()) 
                    {
                        console.warn(
                            `Path "${path}" or key "${key}" must contain string "${userPub}".`,
                        );
                        return false;
                    }

                    // path is allowed, but is there any WRITE block? Check it out
                    // "data.wb" = path to the WRITE block
                    const writeBlockPath =
                        isObject(data.wb) && isString(data.wb['#'])
                            ? data.wb['#']
                            : isString(data.wb)
                                ? data.wb
                                : null;

                    if (isString(writeBlockPath)) 
                    {
                        let root: TGLink | TGClient = client;

                        // Fix if path doesn't start with certificate
                        if (!writeBlockPath.startsWith('~')) 
                        {
                            root = client.get('~' + soulPub);
                        }

                        // TODO: Might be worth changing
                        const value = await root
                            .get(writeBlockPath)
                            .get(userPub)
                            .promise({ timeout: 1000 });

                        if (value === 1 || value === true) 
                        {
                            console.warn(`Certificant ${userPub} blocked.`);
                            return false;
                        }

                        return data;
                    }

                    return data;
                }
            }

            console.warn('Certificate verification fail.');
            return false;
        }
    }

    return false;
}

export function prep(
    val: any,
    key: string,
    node: TGNode,
    soul: string,
): {
    readonly '#': string;
    readonly '.': string;
    readonly ':': TGValue;
    readonly '>': number;
} 
{
    // prep for signing
    return {
        '#': soul,
        '.': key,
        ':': parse(val),
        '>': (node && node._ && node._['>'] && node._['>'][key]) || 0,
    };
}

export async function hashForSignature(prepped: any): Promise<string> 
{
    const hash = await sha256(
        isString(prepped) ? prepped : JSON.stringify(prepped),
    );
    return hash.toString('hex');
}

export function hashNodeKey(node: TGNode, key: string): Promise<string> 
{
    const val = node && node[key];
    const parsed = parse(val);
    const soul = node && node._ && node._['#'];
    const prepped = prep(parsed, key, node, soul as string);

    return hashForSignature(prepped);
}

export async function signHash(
    hash: string,
    pair: PairBase,
    encoding = DEFAULT_OPTS.encode,
): Promise<string> 
{
    const { pub, priv } = pair;
    const token = jwk(pub, priv);
    const signKey = await crypto.subtle.importKey(
        'jwk',
        token,
        ecdsa.pair,
        false,
        ['sign'],
    );
    const sig = await crypto.subtle.sign(
        ecdsa.sign,
        signKey,
        new Uint8Array(Buffer.from(hash, 'hex')),
    );
    return Buffer.from(sig, 'binary').toString(encoding);
}

export async function sign(data: string, pair: PairBase): Promise<string>;
export async function sign(
    data: string,
    pair: PairBase,
    opt: {
        readonly encode?: string;
        readonly raw?: false;
        readonly check?: {
            readonly m: any;
            readonly s: string;
        };
    },
): Promise<string>;
export async function sign(
    data: string,
    pair: PairBase,
    opt: {
        readonly encode?: string;
        readonly raw: true;
        readonly check?: {
            readonly m: any;
            readonly s: string;
        };
    },
): Promise<{ readonly m: any; readonly s: string }>;
export async function sign(
    data: string,
    pair: PairBase,
    opt = DEFAULT_OPTS,
): Promise<string | { readonly m: any; readonly s: string }> 
{
    if (isUndefined(data)) 
    {
        throw new Error('`undefined` not allowed.');
    }
    const json = parse(data);
    const encoding = opt.encode || DEFAULT_OPTS.encode;
    const checkData = opt.check || json;

    if (
        json &&
        ((json.s && json.m) || (json[':'] && json['~'])) &&
        (await verify(data, pair.pub))
    ) 
    {
        // already signed
        const parsed = parse(checkData);
        if (opt.raw) 
        {
            return parsed;
        }
        return 'SEA' + JSON.stringify(parsed);
    }

    const hash = await hashForSignature(data);
    const sig = await signHash(hash, pair, encoding);
    const r = {
        m: json,
        s: sig,
    };
    if (opt.raw) 
    {
        return r;
    }
    return 'SEA' + JSON.stringify(r);
}

export async function signNodeValue(
    node: TGNode,
    key: string,
    pair: PairBase,
): Promise<{
    readonly ':': TGValue;
    readonly '~': string;
}> 
{
    const data = node[key];
    const json = parse(data);

    if (data && json && ((json.s && json.m) || (json[':'] && json['~']))) 
    {
        // already signed
        return json;
    }

    const hash = await hashNodeKey(node, key);
    const sig = await signHash(hash, pair);

    // console.log({':': parse(node[key]), node, key});

    return {
        ':': parse(node[key]),
        '~': sig,
    };
}

export async function signNode(node: TGNode, pair: PairBase): Promise<TGNode> 
{
    const signedNode: TGNode = {
        _: node._,
    };
    const soul = node._ && node._['#'];

    for (const key in node) 
    {
        if (key === '_') 
        {
            continue;
        }
        if (key === 'pub' /*|| key === "alias"*/ && soul === `~${pair.pub}`) 
        {
            // Special case
            signedNode[key] = node[key];
            continue;
        }
        signedNode[key] = JSON.stringify(await signNodeValue(node, key, pair));
    }
    return signedNode;
}

export async function signGraph(
    client: TGClient,
    graph: TGGraphData,
    pair: PairBase,
    putOpt?: TGOptionsPut,
    fullPath?: string[],
): Promise<TGGraphData> 
{
    const modifiedGraph = { ...graph };
    fullPath = Array.isArray(fullPath) ? [...fullPath].reverse() : [];

    for (const soul in graph) 
    {
        if (!soul) 
        {
            continue;
        }

        const node = graph[soul];
        if (!node) 
        {
            continue;
        }

        const soulPub = pubFromSoul(soul);

        // if writing to own graph, just allow it
        if (soulPub === pair.pub) 
        {
            modifiedGraph[soul] = await signNode(node, pair);
        }
        // if writing to other's graph, check if cert exists then try to inject cert into put, also inject self pub so that everyone can verify the put
        else if (soulPub && check(putOpt?.opt?.cert)) 
        {
            const cert = parse(putOpt?.opt?.cert);

            // even if cert exists, we must verify it
            if (
                cert &&
                cert.m &&
                cert.s &&
                (await verifyCertificate(
                    client,
                    cert,
                    pair.pub,
                    soulPub,
                    fullPath,
                ))
            ) 
            {
                modifiedGraph[soul] = await signNode(node, pair);
            }
        }
    }

    return modifiedGraph;
}

export function graphSigner(
    client: TGClient,
    pair: PairBase,
): (
    graph: TGGraphData,
    existingGraph: any,
    putOpt?: TGOptionsPut,
    fullPath?: string[],
) => Promise<TGGraphData> 
{
    return (
        graph: TGGraphData,
        existingGraph: any,
        putOpt?: TGOptionsPut,
        fullPath?: string[],
    ) => signGraph(client, graph, pair, putOpt, fullPath);
}
