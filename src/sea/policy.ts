import { isObject, isString } from '@topgunbuild/typed';
import { IPolicy, IPolicyLex, TGPolicyLexUIOptions, TGPolicyOptions } from '../types';
import { matchPolicy } from '../utils/match-policy';

function mapPolicyLex(
    options: TGPolicyLexUIOptions|undefined,
): IPolicyLex|null
{
    if (!isObject(options))
    {
        return null;
    }

    const lex = Object.keys(options).reduce((accum, key) =>
    {
        const value = options[key];

        switch (key)
        {
        case 'greaterThan':
            accum['>'] = value;
            break;

        case 'lessThan':
            accum['<'] = value;
            break;

        case 'equals':
            accum['='] = value;
            break;

        case 'startsWith':
            accum['*'] = value;
            break;

        case 'pubInPatch':
            accum['+'] = '*';
            break;
        }

        return accum;
    }, {});

    return Object.keys(lex).length > 0 ? lex : null;
}

export function createPolicy(options: TGPolicyOptions): IPolicyLex|IPolicy
{
    const keyPolicy  = mapPolicyLex(options.key);
    const pathPolicy = mapPolicyLex(options.path);

    if (!keyPolicy && !pathPolicy)
    {
        return mapPolicyLex(options) || {};
    }

    const policy: IPolicy = {};

    if (keyPolicy)
    {
        policy['.'] = keyPolicy;
    }
    if (pathPolicy)
    {
        policy['#'] = pathPolicy;
    }

    return policy;
}

export class Policy
{
    private readonly policy: IPolicyLex|IPolicy;
    private readonly path: string;
    private readonly key: string;
    private readonly fullPath: string;
    private readonly certificant: string;

    /**
     * Constructor
     */
    constructor(
        policy: IPolicyLex|IPolicy,
        path: string,
        key: string,
        certificant: string,
    )
    {
        this.policy      = policy;
        this.path        = path || '';
        this.key         = key || '';
        this.fullPath    = this.path + '/' + this.key;
        this.certificant = certificant || '';
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Accessors
    // -----------------------------------------------------------------------------------------------------

    get pathPolicy(): IPolicyLex|undefined
    {
        return this.policy['#'];
    }

    get keyPolicy(): IPolicyLex|undefined
    {
        return this.policy['.'];
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    match(): boolean
    {
        let valid = true;
        if (this.hasPathPolicy() && !matchPolicy(this.path, this.policy['#']))
        {
            valid = false;
        }
        if (this.hasKeyPolicy() && matchPolicy(this.key, this.policy['.']))
        {
            valid = false;
        }
        if (this.hasPlainPolicy())
        {
            valid = matchPolicy(this.fullPath, this.policy as IPolicyLex);
        }

        return valid;
    }

    hasCertificatePathError(): boolean
    {
        let valid = true;

        if (
            this.needCertInPath(this.keyPolicy) &&
            !this.key.includes(this.certificant)
        )
        {
            valid = false;
        }
        if (
            this.needCertInPath(this.pathPolicy) &&
            !this.path.includes(this.certificant)
        )
        {
            valid = false;
        }
        if (
            this.needCertInPath(this.policy as IPolicyLex) &&
            !this.fullPath.includes(this.certificant)
        )
        {
            valid = false;
        }

        return !valid;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private needCertInPath(options: IPolicyLex|string|undefined): boolean
    {
        return isObject(options) && options['+'] === '*';
    }

    private hasPlainPolicy(): boolean
    {
        return (
            isString(this.policy['=']) ||
            isString(this.policy['*']) ||
            isString(this.policy['>']) ||
            isString(this.policy['<'])
        );
    }

    private hasPathPolicy(): boolean
    {
        return this.hasPolicy(this.pathPolicy);
    }

    private hasKeyPolicy(): boolean
    {
        return this.hasPolicy(this.keyPolicy);
    }

    private hasPolicy(options: IPolicyLex|string|undefined): boolean
    {
        return isString(options) || isObject(options);
    }
}
