import { isObject, isString, isNumber } from 'topgun-typed';
import { sign } from './sign';
import { IPolicy } from '../types/policy';

const DEFAULT_POLICY: IPolicy = {};
const DEFAULT_OPTS: {
    readonly block?: any;
    readonly raw?: boolean;
    readonly expiry?: string|number;
}                             = {};
type WhoCertify = '*'|string|string[]|{pub: string}|{pub: string}[];

export async function certify(
    who: WhoCertify,
    policy: IPolicy,
    authority: {priv: string; pub: string},
): Promise<string>;
export async function certify(
    who: WhoCertify,
    policy: IPolicy,
    authority: {priv: string; pub: string},
    opt: {
        readonly block?: any;
        readonly expiry?: string|number;
        readonly raw?: false;
    },
): Promise<string>;
export async function certify(
    who: WhoCertify,
    policy: IPolicy,
    authority: {priv: string; pub: string},
    opt: {
        readonly block?: any;
        readonly expiry?: string|number;
        readonly raw: true;
    },
): Promise<{readonly m: any; readonly s: string}>;
export async function certify(
    who: WhoCertify,
    policy: IPolicy = DEFAULT_POLICY,
    authority: {priv: string; pub: string},
    opt             = DEFAULT_OPTS,
): Promise<string|{readonly m: any; readonly s: string}|undefined>
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
                            : null;
                }
                who.map((certificant) =>
                {
                    if (isString(certificant))
                    {
                        data.push(certificant);
                    }
                    else if (isObject(certificant) && certificant.pub)
                    {
                        data.push(certificant.pub);
                    }
                });
            }
            else if (isObject(who) && who.pub)
            {
                return who.pub;
            }
            return data.length > 0 ? data : null;
        }
        return null;
    })() as WhoCertify;

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
    const readPolicy  =
              !Array.isArray(policy) && isObject(policy) && policy['read']
                  ? policy['read']
                  : null;
    const writePolicy =
              !Array.isArray(policy) && isObject(policy) && policy['write']
                  ? policy['write']
                  : isString(policy) ||
                  Array.isArray(policy) ||
                  (policy && policy['+']) ||
                  policy['#'] ||
                  policy['.'] ||
                  policy['='] ||
                  policy['*'] ||
                  policy['>'] ||
                  policy['<']
                      ? policy
                      : null;

    // We can now use 1 key: block

    const block      = isObject(opt) ? opt.block || {} : {};
    const readBlock  =
              block.read && (isString(block.read) || (block.read || {})['#'])
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
