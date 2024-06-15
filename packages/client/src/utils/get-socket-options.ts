import { SocketClientOptions } from '@topgunbuild/socket';
import { isObject, isString } from '@topgunbuild/utils';

export function getSocketOptions(peer: string|SocketClientOptions): SocketClientOptions
{
    if (isString(peer))
    {
        const url                            = new URL(peer);
        const options: SocketClientOptions = {
            hostname: url.hostname,
            secure  : url.protocol.includes('https'),
        };

        if (url.port.length > 0)
        {
            options.port = Number(url.port);
        }

        return options;
    }
    else if (isObject(peer))
    {
        return peer;
    }

    return null;
}
