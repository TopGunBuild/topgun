import { randomId, windowOrGlobal } from '@topgunbuild/utils';
import { SocketClientOptions } from './types';
import { InvalidArgumentsError } from '../errors';
import { ClientSocket } from './client';

function isUrlSecure(): boolean
{
    return windowOrGlobal.location && windowOrGlobal.location.protocol === 'https:';
}

function getPort(options: SocketClientOptions, isSecureDefault?: boolean): number
{
    const isSecure = options.secure == null ? isSecureDefault : options.secure;
    return (
        options.port ||
        (windowOrGlobal.location && windowOrGlobal.location.port
            ? parseFloat(windowOrGlobal.location.port)
            : isSecure
                ? 443
                : 80)
    );
}

export function create(options: SocketClientOptions): ClientSocket
{
    options = options || {};

    if (options.host && !options.host.match(/[^:]+:\d{2,5}/))
    {
        throw new InvalidArgumentsError(
            'The host option should include both' +
            ' the hostname and the port number in the format "hostname:port"'
        );
    }

    if (options.host && options.hostname)
    {
        throw new InvalidArgumentsError(
            'The host option should already include' +
            ' the hostname and the port number in the format "hostname:port"' +
            ' - Because of this, you should never use host and hostname options together'
        );
    }

    if (options.host && options.port)
    {
        throw new InvalidArgumentsError(
            'The host option should already include' +
            ' the hostname and the port number in the format "hostname:port"' +
            ' - Because of this, you should never use host and port options together'
        );
    }

    const isSecureDefault = isUrlSecure();

    const opts: SocketClientOptions = {
        clientId: randomId(),
        port    : getPort(options, isSecureDefault),
        hostname: windowOrGlobal.location && windowOrGlobal.location.hostname || 'localhost',
        secure  : isSecureDefault,
    };

    Object.assign(opts, options);

    return new ClientSocket(opts);
}

