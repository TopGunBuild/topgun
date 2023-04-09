import { sign } from './sign';
import { isObject } from '../utils/is-object';
import { isString } from '../utils/is-string';
import { isNumber } from '../utils/is-number';
import { IPolicy } from '../types/policy';

const DEFAULT_POLICY: IPolicy = {};
const DEFAULT_OPTS: {
    readonly block?: any;
    readonly raw?: boolean;
    readonly expiry?: string|number;
}                             = {};
type WhoCertify = '*' | string | string[] | { pub: string } | { pub: string }[];

/**
 * This is to certify that a group of "who" can "put" anything at a group of matched "paths" to the certificate authority's graph
 * A Certificate is like a Signature. No one knows who (authority) created/signed a cert until you put it into their graph.
 *
 * @param who '*' or a String (Bob.pub) || an Object that contains "pub" as a key || an array of [object || string].
 * These people will have the rights.
 *
 * @param {{}} policy A string ('inbox'), or a LEX object {'*': 'inbox'}, or an Array of RAD/LEX objects or strings.
 * RAD/LEX object can contain key "?" with indexOf("*") > -1 to force key equals certificant pub.
 * This rule is used to check against soul+'/'+key using Gun.text.match or String.match.
 *
 * @param authority Key pair or priv of the certificate authority.
 *
 * @param {{}} opt If opt.expiry (a timestamp) is set, SEA won't sync data after opt.expiry. If opt.block is set, SEA will look for block before syncing.
 *
 *  @returns {Promise<void>}
 */
export async function certify(
    who: '*' | string | string[] | { pub: string } | { pub: string }[],
    policy: IPolicy,
    authority: { priv: string; pub: string },
): Promise<string>
export async function certify(
    who: '*' | string | string[] | { pub: string } | { pub: string }[],
    policy: IPolicy,
    authority: { priv: string; pub: string },
    opt: {
        readonly block?: any;
        readonly expiry?: string|number;
        readonly raw?: false;
    }
): Promise<string>
export async function certify(
    who: '*' | string | string[] | { pub: string } | { pub: string }[],
    policy: IPolicy,
    authority: { priv: string; pub: string },
    opt: {
        readonly block?: any;
        readonly expiry?: string|number;
        readonly raw: true;
    }
): Promise<{readonly m: any; readonly s: string}>
export async function certify(
    who: '*' | string | string[] | { pub: string } | { pub: string }[],
    policy: IPolicy = DEFAULT_POLICY,
    authority: { priv: string; pub: string },
    opt             = DEFAULT_OPTS
): Promise<string|{readonly m: any; readonly s: string}|undefined>
{
    try
    {
        if (!isObject(opt))
        {
            opt = DEFAULT_OPTS;
        }

        who = (() =>
        {
            const data: string[] = [];
            if (who)
            {
                if (isString(who) && who.includes('*'))
                {
                    return '*';
                }
                if (Array.isArray(who) && who.some(e => e === '*'))
                {
                    return '*';
                }
                if (isString(who))
                {
                    return who;
                }
                if (Array.isArray(who))
                {
                    if (who.length === 1 && who[0])
                    {
                        return isObject(who[0]) && who[0].pub
                            ? who[0].pub
                            : isString(who[0])
                                ? who[0]
                                : null
                    }
                    who.map(certificant =>
                    {
                        if (isString(certificant))
                        {
                            data.push(certificant);
                        }
                        else if (isObject(certificant) && certificant.pub)
                        {
                            data.push(certificant.pub);
                        }
                    })
                }
                else if (isObject(who) && who.pub)
                {
                    return who.pub;
                }
                return data.length > 0 ? data : null
            }
            return null;
        })();

        if (!who)
        {
            console.log('No certificant found.');
            return;
        }

        const expiry      = isString(opt?.expiry)
            ? parseFloat(opt.expiry)
            : isNumber(opt?.expiry)
                ? opt.expiry
                : null;
        const readPolicy  = !Array.isArray(policy) && isObject(policy) && policy['read']
            ? policy['read']
            : null;
        const writePolicy = !Array.isArray(policy) && isObject(policy) && policy['write']
            ? policy['write']
            : isString(policy)
            || Array.isArray(policy)
            || (policy && policy['+'] || policy['#'] || policy['.'] || policy['='] || policy['*'] || policy['>'] || policy['<'])
                ? policy
                : null;

        // We can now use 1 key: block

        const block      = isObject(opt)
            ? opt.block || {}
            : {};
        const readBlock  = block.read && (isString(block.read) || (block.read || {})['#'])
            ? block.read
            : null;
        const writeBlock = isString(block)
            ? block
            : block.write && (isString(block.write) || block.write['#'])
                ? block.write
                : null;

        if (!readPolicy && !writePolicy)
        {
            console.log('No policy found.');
            return;
        }

        // reserved keys: c, e, r, w, rb, wb
        const data = JSON.stringify({
            c: who,
            ...(expiry ? { e: expiry } : {}), // inject expiry if possible
            ...(readPolicy ? { r: readPolicy } : {}), // "r" stands for read, which means read permission.
            ...(writePolicy ? { w: writePolicy } : {}), // "w" stands for write, which means write permission.
            ...(readBlock ? { rb: readBlock } : {}), // inject READ block if possible
            ...(writeBlock ? { wb: writeBlock } : {}), // inject WRITE block if possible
        });

        const certificate = await sign(data, authority, { raw: true });

        if (opt.raw)
        {
            return certificate;
        }

        return 'SEA' + JSON.stringify(certificate);
    }
    catch (e)
    {
        console.log(e);
    }
}