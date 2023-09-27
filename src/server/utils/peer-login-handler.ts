import { TGSocket } from '@topgunbuild/socket/server';
import { isString } from '@topgunbuild/typed';
import { TGEncryptData } from '../../types';
import { decrypt, work } from '../../sea';

/**
 * Authenticate a peer connection
 */
export async function peerLoginHandler(
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
)
{
    const data = request.data;

    if (!data?.challenge || !data?.data)
    {
        request.end('Missing peer auth info');
        return;
    }

    const [socketId, timestampStr] = data.challenge.split('/');
    const timestamp                = parseInt(timestampStr, 10);

    if (!socketId || socketId !== socket.id)
    {
        request.error(new Error('Socket ID doesn\'t match'));
        return;
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
        }
        else
        {
            request.end('Invalid auth key');
        }
    }
    catch (err)
    {
        request.end('Invalid auth key');
    }
}