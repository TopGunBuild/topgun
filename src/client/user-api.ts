import { isObject, isString, isFunction } from 'topgun-typed';
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
import { assertCredentials, assertNotEmptyString } from '../utils/assert';
import { TGLink } from './link';
import { isValidCredentials } from '../utils/is-valid-credentials';
import { getSessionStorage } from '../utils/session-storage';

let sessionStorage: TGSupportedStorage;
let sessionStorageKey: string;

export class TGUserApi
{
    readonly #client: TGClient;
    #signMiddleware?: (
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
        _sessionStorage: TGSupportedStorage|undefined|boolean,
        _sessionStorageKey: string|undefined
    )
    {
        this.#client      = client;
        sessionStorage    = getSessionStorage(_sessionStorage);
        sessionStorageKey = _sessionStorageKey;
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
    ): Promise<TGUserCredentials>
    {
        try
        {
            const credentials = await createUser(this.#client, alias, password);
            await this.useCredentials(credentials);
            if (isFunction(cb))
            {
                cb(credentials);
            }
            return credentials;
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
    ): Promise<TGUserCredentials|undefined>;
    async auth(
        alias: string,
        password: string,
        cb?: TGAuthCallback,
        _opt?: AuthOptions,
    ): Promise<TGUserCredentials|undefined>;
    async auth(
        aliasOrPair: string|Pair,
        passwordOrCallback: string|TGAuthCallback,
        optionsOrCallback?: TGAuthCallback|AuthOptions,
        maybeOptions?: AuthOptions,
    ): Promise<TGUserCredentials|undefined>
    {
        const cb = isFunction(optionsOrCallback)
            ? optionsOrCallback
            : isFunction(passwordOrCallback)
                ? passwordOrCallback
                : null;

        try
        {
            await this.recoverCredentials();

            let credentials: TGUserCredentials;

            if (
                isObject(aliasOrPair) &&
                (aliasOrPair.pub || aliasOrPair.epub)
            )
            {
                const pair    = aliasOrPair;
                const options = optionsOrCallback as AuthOptions;

                credentials = await authenticate(this.#client, pair as Pair, options);

                await this.useCredentials(credentials);

                if (isFunction(cb))
                {
                    cb(credentials);
                }

                return credentials;
            }
            else if (isString(aliasOrPair) && isString(passwordOrCallback))
            {
                const alias    = aliasOrPair;
                const password = passwordOrCallback;
                const options  = maybeOptions;

                credentials = await authenticate(
                    this.#client,
                    alias,
                    password,
                    options,
                );
                await this.useCredentials(credentials);

                if (isFunction(cb))
                {
                    cb(credentials);
                }

                return credentials;
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
        if (this.#signMiddleware)
        {
            this.#removeCredentials();
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
            ? this.#client.get(`~${this.is.pub}`).get(soul)
            : this.#client.get(`~${this.#client.WAIT_FOR_USER_PUB}`).get(soul);
    }

    /**
     * Recovers the credentials from LocalStorage
     */
    async recoverCredentials(): Promise<void>
    {
        if (sessionStorage)
        {
            const maybeSession = await getItemAsync(
                sessionStorage,
                sessionStorageKey,
            );

            if (maybeSession !== null)
            {
                if (isValidCredentials(maybeSession))
                {
                    await this.useCredentials(maybeSession);
                }
                else
                {
                    await this.#removeCredentials();
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
        this.#signMiddleware = graphSigner(this.#client, {
            priv: credentials.priv,
            pub : credentials.pub,
        });
        this.#client.graph.use(this.#signMiddleware, 'write');
        this.is = {
            alias: credentials.alias,
            pub  : credentials.pub,
        };

        if (this.is && this.is.pub)
        {
            await this.#authSuccess(credentials);
        }

        return this.is;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    async #authSuccess(credentials: TGUserCredentials): Promise<void>
    {
        await Promise.all([
            this.#authConnectors(credentials),
            this.#persistCredentials(credentials)
        ]);

        this.#client.emit('auth', {
            alias: credentials.alias,
            pub  : credentials.pub,
        });
    }

    async #authConnectors(credentials: TGUserCredentials): Promise<any>
    {
        await this.#client.graph.eachConnector(async (connector) =>
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

    async #persistCredentials(
        credentials: TGUserCredentials,
    ): Promise<void>
    {
        if (sessionStorage)
        {
            await setItemAsync(
                sessionStorage,
                sessionStorageKey,
                credentials,
            );
        }
    }

    async #removeCredentials(): Promise<void>
    {
        if (sessionStorage)
        {
            await removeItemAsync(
                sessionStorage,
                sessionStorageKey,
            );
        }
    }
}
