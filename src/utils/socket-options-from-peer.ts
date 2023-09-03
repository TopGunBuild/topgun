import { TGSocketClientOptions } from '@topgunbuild/socket/client';
import { isObject, isString } from '@topgunbuild/typed';

export function socketOptionsFromPeer(peer: string|TGSocketClientOptions): TGSocketClientOptions
{
    if (isString(peer))
    {
        const url                            = new URL(peer);
        const options: TGSocketClientOptions = {
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
