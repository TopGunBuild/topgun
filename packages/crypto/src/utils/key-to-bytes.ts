import { base58 } from '@scure/base';
import { textEncoder } from '@topgunbuild/textencoder';

type Encoding = 'base58'|'utf8';

export const keyToBytes = (value: string, encoding: Encoding = 'base58'): Uint8Array =>
{
    switch (encoding)
    {
        case 'utf8':
            return textEncoder.encode(value);

        case 'base58':
            return base58.decode(value);

        default:
            throw new Error(`Unknown encoding: ${encoding as string}`);
    }
};

