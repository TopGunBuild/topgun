import WebCrypto from 'topgun-webcrypto';
import { keyToJwk } from './settings';
import { sha256 } from './sha256';
import { random } from '../utils/random';

export async function importAesKey(key: string, salt: any): Promise<any>
{
    const combo  = key + (salt || random(8)).toString('utf8');
    const hash   = await sha256(combo);
    const jwkKey = keyToJwk(hash);
    return WebCrypto.subtle.importKey('jwk', jwkKey, 'AES-GCM', false, [
        'encrypt',
        'decrypt',
    ]);
}
