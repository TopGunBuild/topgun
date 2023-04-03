import { authenticate, createUser, graphSigner } from '../sea';
import { Client } from './client';
import { Event } from './control-flow/event';
import { SocketConnector } from '../socket-connector';
import { getItemAsync, removeItemAsync, setItemAsync } from '../utils/storage-helpers';
import { Pair } from '../sea/pair';
import { AuthOptions } from '../sea/authenticate';
import { AuthCallback, SupportedStorage, OptionsPut, UserCredentials, UserReference } from '../types';
import { isObject } from '../utils/is-object';
import { isString } from '../utils/is-string';
import { isFunction } from '../utils/is-function';
import { LexLink } from './lex-link';

const DEFAULT_CREATE_OPTS = {};

export class UserApi
{
    private readonly _client: Client;
    private readonly _persistSession: boolean;
    private readonly _sessionStorage: SupportedStorage;
    private readonly _sessionStorageKey: string;
    private readonly _authEvent: Event<UserReference>;
    private _signMiddleware?: (graph: any, existingGraph: any, putOpt?: OptionsPut) => Promise<any>;
    private _credentials: UserCredentials;
    public is?: UserReference;

    /**
     * Constructor
     */
    constructor(
        client: Client,
        persistSession: boolean,
        sessionStorage: SupportedStorage,
        sessionStorageKey: string,
        authEvent: Event<UserReference>
    )
    {
        this._authEvent         = authEvent;
        this._client            = client;
        this._persistSession    = persistSession;
        this._sessionStorage    = sessionStorage;
        this._sessionStorageKey = sessionStorageKey;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Creates a new user and calls callback upon completion.
     */
    public async create(
        alias: string,
        password: string,
        cb?: AuthCallback,
        _opt = DEFAULT_CREATE_OPTS
    ): Promise<UserReference>
    {
        try
        {
            const user = await createUser(this._client, alias, password);
            const ref  = this.useCredentials(user);
            if (cb)
            {
                cb(ref)
            }
            return ref
        }
        catch (err)
        {
            if (cb)
            {
                cb({ err })
            }
            throw err
        }
    }

    /**
     * Authenticates a user, previously created via User.create.
     */
    public async auth(
        pair: Pair,
        cb?: AuthCallback,
        _opt?: AuthOptions
    ): Promise<UserReference>
    public async auth(
        alias: string,
        password: string,
        cb?: AuthCallback,
        _opt?: AuthOptions
    ): Promise<UserReference>
    public async auth(
        aliasOrPair: string|Pair,
        passwordOrCallback: string|AuthCallback,
        optionsOrCallback?: AuthCallback|AuthOptions,
        maybeOptions?: AuthOptions
    ): Promise<UserReference>
    {
        const cb = isFunction(optionsOrCallback)
            ? optionsOrCallback
            : isFunction(passwordOrCallback)
                ? passwordOrCallback
                : null;

        try
        {
            await this.recoverCredentials();

            let user: UserCredentials;

            if (isObject(aliasOrPair) && (aliasOrPair.pub || aliasOrPair.epub))
            {
                const pair    = aliasOrPair;
                const options = optionsOrCallback as AuthOptions;

                user = await authenticate(this._client, pair as Pair, options);
            }
            else if (isString(aliasOrPair) && isString(passwordOrCallback))
            {
                const alias    = aliasOrPair;
                const password = passwordOrCallback;
                const options  = maybeOptions;

                user = await authenticate(this._client, alias, password, options);
            }

            const ref = this.useCredentials(user);

            if (cb)
            {
                cb(ref)
            }
            return ref
        }
        catch (err)
        {
            if (cb)
            {
                cb({ err })
            }
            throw err
        }
    }

    /**
     * Log out currently authenticated user
     */
    public leave(): UserApi
    {
        if (this._signMiddleware)
        {
            // this._removeCredentials();
            this._client.graph.unuse(this._signMiddleware, 'write');
            this._signMiddleware = undefined;
            this.is              = undefined;
        }

        return this
    }

    /**
     * Traverse a location in the graph
     */
    public get(soul: string): LexLink|undefined
    {
        return this.is && this._client.get(`~${this.is.pub}/${soul}`);
    }

    /**
     * Recovers the credentials from LocalStorage
     */
    public async recoverCredentials(): Promise<void>
    {
        if (this._persistSession)
        {
            const maybeSession = await getItemAsync(this._sessionStorage, this._sessionStorageKey);

            if (maybeSession !== null)
            {
                if (this._isValidCredentials(maybeSession))
                {
                    this.useCredentials(maybeSession);
                }
                else
                {
                    await this._removeCredentials()
                }
            }
        }
    }

    /**
     * Authenticates a user by credentials
     */
    public useCredentials(
        credentials: UserCredentials
    ): {
        readonly alias: string
        readonly pub: string
    }
    {
        this.leave();
        this._signMiddleware = graphSigner(this._client, {
            priv: credentials.priv,
            pub : credentials.pub
        });
        this._client.graph.use(this._signMiddleware, 'write');
        this.is = {
            alias: credentials.alias,
            pub  : credentials.pub
        };

        if (this.is && this.is.pub)
        {
            this._authSuccess(credentials);
        }

        return this.is;
    }

    public pair(): UserCredentials
    {
        return this._credentials;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _authSuccess(credentials: UserCredentials): void
    {
        this._credentials = credentials;
        this._authEvent.trigger(this.is);
        this._authConnectors(credentials);
        this._persistCredentials(credentials);
    }

    private _authConnectors(credentials: UserCredentials): void
    {
        this._client.graph.eachConnector(connector =>
        {
            if (connector.name === 'SocketConnector')
            {
                (connector as SocketConnector).authenticate(credentials.pub, credentials.priv);
            }
        });
    }

    private async _persistCredentials(credentials: UserCredentials): Promise<void>
    {
        if (this._persistSession)
        {
            await setItemAsync(this._sessionStorage, this._sessionStorageKey, credentials);
        }
    }

    private async _removeCredentials(): Promise<void>
    {
        if (this._persistSession)
        {
            await removeItemAsync(this._sessionStorage, this._sessionStorageKey);
        }
        else
        {
            this._credentials = null;
        }
    }

    private _isValidCredentials(maybeSession: unknown): maybeSession is UserCredentials
    {
        return isObject(maybeSession) &&
            'priv' in maybeSession &&
            'epriv' in maybeSession &&
            'alias' in maybeSession &&
            'pub' in maybeSession &&
            'epub' in maybeSession;
    }
}
