import { base58 } from '@scure/base';
import { type Base58Keypair, type KeyPair } from '@topgunbuild/types';

export const keypairToBase58 = (keypair: KeyPair): Base58Keypair => ({
    publicKey: base58.encode(keypair.publicKey),
    secretKey: base58.encode(keypair.privateKey),
});
