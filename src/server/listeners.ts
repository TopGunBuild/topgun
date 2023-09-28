import { isString } from '@topgunbuild/typed';
import { TGSocketServer, TGSocket } from '@topgunbuild/socket/server';
import { TGEncryptData, TGMessage } from '../types';
import { decrypt, verify, work } from '../sea';
import { TGLoggerType } from '../logger';
import { TGServerOptions } from './server-options';

export class Listeners
{
    readonly inboundPeerConnections: Map<string, TGSocket>;

    /**
     * Constructor
     */
    constructor(
        private readonly gateway: TGSocketServer,
        private readonly logger: TGLoggerType,
        private readonly options: TGServerOptions,
        private readonly serverName: string
    )
    {
        this.inboundPeerConnections = new Map<string, TGSocket>();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Logs disconnection
     */
    async disconnectionListener(): Promise<void>
    {
        for await (const { socket, code, reason } of this.gateway.listener('disconnection'))
        {
            // this.logger.log(`socket ${socket.id} disconnected with code ${code} due to ${reason}`);

            if (this.inboundPeerConnections.has(socket.id))
            {
                this.inboundPeerConnections.delete(socket.id);
            }
        }
    }

    /**
     * Set up a loop to handle websocket connections.
     */
    async connectionListener(): Promise<void>
    {
        for await (const { socket } of this.gateway.listener('connection'))
        {
            this.#authListener(socket);
            this.#peerAutListener(socket);
        }
    }

    /**
     * Logs when the server is ready
     */
    async readyListener(): Promise<void>
    {
        for await (const data of this.gateway.listener('ready'))
        {
            this.logger.log('TopGun Server is ready');
        }
    }

    /**
     * Logs out errors
     */
    async errorListener(): Promise<void>
    {
        for await (const { error } of this.gateway.listener('error'))
        {
            this.logger.error(error);
        }
    }

    /**
     * Publish changes to peers
     */
    publishChangeLog(message: TGMessage): void
    {
        this.inboundPeerConnections.forEach((socket: TGSocket) =>
        {
            if (socket.state === socket.OPEN && socket.authState === socket.AUTHENTICATED)
            {
                socket.transmit('#publish', {
                    channel: 'topgun/changelog',
                    data   : message
                });
            }
        });
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * RPC listener for a socket's login
     */
    async #authListener(socket: TGSocket): Promise<void>
    {
        for await (const request of socket.procedure('login'))
        {
            this.#loginHandler(socket, request);
        }
    }

    /**
     * RPC listener for a socket's login
     */
    async #peerAutListener(socket: TGSocket): Promise<void>
    {
        for await (const request of socket.procedure('peerLogin'))
        {
            this.#peerLoginHandler(socket, request, this.options.peerSecretKey, this.serverName).then((isAuth) =>
            {
                if (isAuth)
                {
                    this.inboundPeerConnections.set(socket.id, socket);
                }
            });
        }
    }

    /**
     * Authenticate a client connection for extra privileges
     */
    async #loginHandler(
        socket: TGSocket,
        request: {
            data: {
                pub: string;
                proof: {
                    m: string;
                    s: string;
                };
            },
            end: (reason?: string) => void,
            error: (error?: Error) => void
        },
    ): Promise<void>
    {
        const data = request.data;

        if (!data.pub || !data.proof)
        {
            request.end('Missing login info');
            return;
        }

        try
        {
            const [socketId, timestampStr] = data.proof.m.split('/');
            const timestamp                = parseInt(timestampStr, 10);

            if (!socketId || socketId !== socket.id)
            {
                request.error(new Error('Socket ID doesn\'t match'));
                return;
            }

            const isVerified = await verify(data.proof, data.pub);

            if (isVerified)
            {
                await socket.setAuthToken({
                    pub: data.pub,
                    timestamp,
                });
                request.end();
            }
            else
            {
                request.end('Invalid login');
            }
        }
        catch (err)
        {
            request.end('Invalid login');
        }
    }

    /**
     * Authenticate a peer connection
     */
    async #peerLoginHandler(
        socket: TGSocket,
        request: {
            data: {
                challenge: string;
                data: TGEncryptData;
            },
            end: (reason?: string) => void,
            error: (error?: Error) => void
        },
        peerSecretKey: string,
        serverName: string
    ): Promise<boolean>
    {
        const data = request.data;

        if (!data?.challenge || !data?.data)
        {
            request.end('Missing peer auth info');
            return false;
        }

        const [socketId, timestampStr] = data.challenge.split('/');
        const timestamp                = parseInt(timestampStr, 10);

        if (!socketId || socketId !== socket.id)
        {
            request.error(new Error('Socket ID doesn\'t match'));
            return false;
        }

        try
        {
            const hash      = await work(data.challenge, peerSecretKey);
            const decrypted = await decrypt<{peerUri: string}>(data.data, hash);

            if (isString(decrypted?.peerUri))
            {
                await socket.setAuthToken({
                    peerUri: decrypted.peerUri,
                    serverName,
                    timestamp,
                });
                request.end();
                return true;
            }
            else
            {
                request.end('Invalid auth key');
                return false;
            }
        }
        catch (err)
        {
            request.end('Invalid auth key');
            return false;
        }
    }
}
