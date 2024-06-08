import { SocketClientOptions } from '@topgunbuild/socket';
import { Store } from '@topgunbuild/store';
import { Ed25519Keypair } from '@topgunbuild/crypto';

export interface ClientOptions
{
    peers?: (string|SocketClientOptions)[];
    store?: Store;
    directory?: string;
    identity?: Ed25519Keypair;
}
