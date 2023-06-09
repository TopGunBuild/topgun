import { isObject, isString, isFunction, object, fn } from 'topgun-typed';
import { authenticate, createUser, graphSigner } from '../sea';
import { TGClient } from './client';
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
import { DEFAULT_OPTIONS } from './client-options';
import { assertCredentials, assertNotEmptyString } from '../utils/assert';
import { TGLink } from './link';
import { localStorageAdapter } from '../utils/local-storage';

const storageStruct = object({
    getItem   : fn(),
    setItem   : fn(),
    removeItem: fn()
});

export class TGUserApi
{
    private readonly _client: TGClient;
    private readonly _sessionStorage: TGSupportedStorage;
    private readonly _sessionStorageKey: string;
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
        sessionStorage: TGSupportedStorage|undefined|boolean,
        sessionStorageKey: string|undefined
    )
    {
        this._client            = client;
        this._sessionStorage    = !sessionStorage
            ? null
            : storageStruct(sessionStorage).ok
                ? (sessionStorage as TGSupportedStorage)
                : localStorageAdapter;
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
            const ref  = await this.useCredentials(user);
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
                ref  = await this.useCredentials(user);

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
                ref  = await this.useCredentials(user);

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
            this._removeCredentials();
            this.is = undefined;
        }

        return this;
    }

    /**
     * Traverse a location in the graph
     */
    get(soul: string): TGLink
    {
        soul = assertNotEmptyString(soul);
        return !!this.is
            ? this._client.get(`~${this.is.pub}/${soul}`)
            : this._client.get(`~${this._client.WAIT_FOR_USER_PUB}/${soul}`);
    }

    /**
     * Recovers the credentials from LocalStorage
     */
    async recoverCredentials(): Promise<void>
    {
        if (this._sessionStorage)
        {
            const maybeSession = await getItemAsync(
                this._sessionStorage,
                this._sessionStorageKey,
            );

            if (maybeSession !== null)
            {
                if (this._isValidCredentials(maybeSession))
                {
                    await this.useCredentials(maybeSession);
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
    async useCredentials(credentials: TGUserCredentials): Promise<{
        readonly alias: string;
        readonly pub: string;
    }>
    {
        credentials = assertCredentials(credentials);

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
            await this._authSuccess(credentials);
        }

        return this.is;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private async _authSuccess(credentials: TGUserCredentials): Promise<void>
    {
        await Promise.all([
            this._authConnectors(credentials),
            this._persistCredentials(credentials)
        ]);

        this._client.emit('auth', {
            alias: credentials.alias,
            pub  : credentials.pub,
        });
    }

    private async _authConnectors(credentials: TGUserCredentials): Promise<any>
    {
        await this._client.graph.eachConnector(async (connector) =>
        {
            if (isFunction(connector?.authenticate))
            {
                await connector.authenticate(
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
        if (this._sessionStorage)
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
        if (this._sessionStorage)
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
