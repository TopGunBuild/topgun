import { TGSocket } from '@topgunbuild/socket/server';
import { verify } from '../../sea';

/**
 * Authenticate a client connection for extra privileges
 */
export async function clientLoginHandler(
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
