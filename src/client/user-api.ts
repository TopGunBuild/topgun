import { isObject, isString, isFunction, isBoolean } from 'topgun-typed';
import { authenticate, createUser, graphSigner } from '../sea';
import { TGClient } from './client';
import { TGEvent } from './control-flow/event';
import { TGWebSocketGraphConnector } from './transports/web-socket-graph-connector';
import {
    getItemAsync,
    removeItemAsync,
    setItemAsync,
} from '../utils/storage-helpers';
import { Pair } from '../sea/pair';
import { AuthOptions } from '../sea/authenticate';
import {
    TGAuthCallback,
    TGSupportedStorage,
    TGOptionsPut,
    TGUserCredentials,
    TGUserReference,
} from '../types';
import { TGLexLink } from './lex-link';
import { DEFAULT_OPTIONS } from './client-options';

export class TGUserApi
{
    private readonly _client: TGClient;
    private readonly _persistSession: boolean;
    private readonly _sessionStorage: TGSupportedStorage;
    private readonly _sessionStorageKey: string;
    private readonly _authEvent: TGEvent<TGUserReference>;
    private _signMiddleware?: (
        graph: any,
        existingGraph: any,
        putOpt?: TGOptionsPut,
    ) => Promise<any>;
    is?: TGUserReference;

    /**
     * Constructor
     */
    constructor(
        client: TGClient,
        persistSession: boolean|undefined,
        sessionStorage: TGSupportedStorage|undefined,
        sessionStorageKey: string|undefined,
        authEvent: TGEvent<TGUserReference>,
    )
    {
        this._authEvent         = authEvent;
        this._client            = client;
        this._persistSession    = isBoolean(persistSession)
            ? persistSession
            : DEFAULT_OPTIONS.persistSession;
        this._sessionStorage    = sessionStorage || DEFAULT_OPTIONS.sessionStorage;
        this._sessionStorageKey = sessionStorageKey || DEFAULT_OPTIONS.sessionStorageKey;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Creates a new user and calls callback upon completion.
     */
    async create(
        alias: string,
        password: string,
        cb?: TGAuthCallback,
    ): Promise<TGUserReference>
    {
        try
        {
            const user = await createUser(this._client, alias, password);
            const ref  = this.useCredentials(user);
            if (cb)
            {
                cb(ref);
            }
            return ref;
        }
        catch (err)
        {
            if (cb)
            {
                cb({ err: err as Error });
            }
            throw err;
        }
    }

    /**
     * Authenticates a user, previously created via User.create.
     */
    async auth(
        pair: Pair,
        cb: TGAuthCallback,
        _opt?: AuthOptions,
    ): Promise<TGUserReference|undefined>;
    async auth(
        alias: string,
        password: string,
        cb?: TGAuthCallback,
        _opt?: AuthOptions,
    ): Promise<TGUserReference|undefined>;
    async auth(
        aliasOrPair: string|Pair,
        passwordOrCallback: string|TGAuthCallback,
        optionsOrCallback?: TGAuthCallback|AuthOptions,
        maybeOptions?: AuthOptions,
    ): Promise<TGUserReference|undefined>
    {
        const cb = isFunction(optionsOrCallback)
            ? optionsOrCallback
            : isFunction(passwordOrCallback)
                ? passwordOrCallback
                : null;

        try
        {
            await this.recoverCredentials();

            let user: TGUserCredentials;
            let ref: TGUserReference;

            if (
                isObject(aliasOrPair) &&
                (aliasOrPair.pub || aliasOrPair.epub)
            )
            {
                const pair    = aliasOrPair;
                const options = optionsOrCallback as AuthOptions;

                user = await authenticate(this._client, pair as Pair, options);
                ref  = this.useCredentials(user);

                if (cb)
                {
                    cb(ref);
                }

                return ref;
            }
            else if (isString(aliasOrPair) && isString(passwordOrCallback))
            {
                const alias    = aliasOrPair;
                const password = passwordOrCallback;
                const options  = maybeOptions;

                user = await authenticate(
                    this._client,
                    alias,
                    password,
                    options,
                );
                ref  = this.useCredentials(user);

                if (cb)
                {
                    cb(ref);
                }

                return ref;
            }
        }
        catch (err)
        {
            if (cb)
            {
                cb({ err: err as Error });
            }
            throw err;
        }
    }

    /**
     * Log out currently authenticated user
     */
    leave(): TGUserApi
    {
        if (this._signMiddleware)
        {
            // this._removeCredentials();
            // this._client.graph.unuse(this._signMiddleware, 'write');
            this._signMiddleware = undefined;
            this.is              = undefined;
        }

        return this;
    }

    /**
     * Traverse a location in the graph
     */
    get(soul: string): TGLexLink|undefined
    {
        return this.is && this._client.get(`~${this.is.pub}/${soul}`);
    }

    /**
     * Recovers the credentials from LocalStorage
     */
    async recoverCredentials(): Promise<void>
    {
        if (this._persistSession)
        {
            const maybeSession = await getItemAsync(
                this._sessionStorage,
                this._sessionStorageKey,
            );

            if (maybeSession !== null)
            {
                if (this._isValidCredentials(maybeSession))
                {
                    this.useCredentials(maybeSession);
                }
                else
                {
                    await this._removeCredentials();
                }
            }
        }
    }

    /**
     * Authenticates a user by credentials
     */
    useCredentials(credentials: TGUserCredentials): {
        readonly alias: string;
        readonly pub: string;
    }
    {
        this.leave();
        this._signMiddleware = graphSigner(this._client, {
            priv: credentials.priv,
            pub : credentials.pub,
        });
        this._client.graph.use(this._signMiddleware, 'write');
        this.is = {
            alias: credentials.alias,
            pub  : credentials.pub,
        };

        if (this.is && this.is.pub)
        {
            this._authSuccess(credentials);
        }

        return this.is;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _authSuccess(credentials: TGUserCredentials): void
    {
        this._authEvent.trigger({
            alias: credentials.alias,
            pub  : credentials.pub,
        });
        this._authConnectors(credentials);
        this._persistCredentials(credentials);
    }

    private _authConnectors(credentials: TGUserCredentials): void
    {
        this._client.graph.eachConnector((connector) =>
        {
            if (connector.name === 'TGWebSocketGraphConnector')
            {
                (connector as TGWebSocketGraphConnector).authenticate(
                    credentials.pub,
                    credentials.priv,
                );
            }
        });
    }

    private async _persistCredentials(
        credentials: TGUserCredentials,
    ): Promise<void>
    {
        if (this._persistSession)
        {
            await setItemAsync(
                this._sessionStorage,
                this._sessionStorageKey,
                credentials,
            );
        }
    }

    private async _removeCredentials(): Promise<void>
    {
        if (this._persistSession)
        {
            await removeItemAsync(
                this._sessionStorage,
                this._sessionStorageKey,
            );
        }
    }

    private _isValidCredentials(
        maybeSession: unknown,
    ): maybeSession is TGUserCredentials
    {
        return (
            isObject(maybeSession) &&
            'priv' in maybeSession &&
            'epriv' in maybeSession &&
            'alias' in maybeSession &&
            'pub' in maybeSession &&
            'epub' in maybeSession
        );
    }
}
