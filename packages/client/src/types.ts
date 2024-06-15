import { SocketClientOptions } from '@topgunbuild/socket';
import { Store } from '@topgunbuild/store';
import { Ed25519Keypair } from '@topgunbuild/crypto';

export interface ClientOptions
{
    peers?: PeerOption[];
    store?: Store;
    directory?: string;
    identity?: Ed25519Keypair;
}


export type MessageCb = (msg: any) => void;
export type PeerOption = string|SocketClientOptions;
