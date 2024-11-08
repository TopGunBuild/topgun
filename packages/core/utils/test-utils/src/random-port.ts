import * as net from 'net';

export function randomPort(opts: { from?: number, range?: number } = {}): Promise<number>
{
    return new Promise((resolve) =>
    {
        _randomPort(opts, port => resolve(port));
    })
}

function _randomPort(opts: { from?: number, range?: number } = {}, cb?: (port: number) => void): void
{
    if (arguments.length == 0)
    {
        throw 'no callback';
    }
    else if (arguments.length == 1)
    {
        cb = arguments[0];
    }
    else
    {
        opts = arguments[0];
        cb   = arguments[arguments.length - 1];
    }

    if (typeof cb != 'function')
    {
        throw 'callback is not a function';
    }

    if (typeof opts != 'object')
    {
        throw 'options is not a object';
    }

    const from  = opts.from > 0 ? opts.from : 15000,
          range = opts.range > 0 ? opts.range : 100,
          port  = from + ~~(Math.random() * range);

    const server = net.createServer();
    server.listen(port, function()
    {
        server.once('close', function()
        {
            cb(port);
        });
        server.close();
    });
    server.on('error', function(err)
    {
        _randomPort(opts, cb);
    });
}
