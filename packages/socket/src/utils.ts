import { windowOrGlobal } from '@topgunbuild/utils';

export function createWebSocket(uri: string, options?: any)
{
    if (windowOrGlobal && typeof windowOrGlobal.WebSocket === 'function')
    {
        return new windowOrGlobal.WebSocket(uri);
    }
    else
    {
        const WebSocket: any = require('ws');
        return new WebSocket(uri, [], options);
    }
}

