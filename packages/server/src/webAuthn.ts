import base64 from '@hexagon/base64';
import { Fido2Lib, Factor } from 'fido2-lib';

export class Fido2
{
    f2l: Fido2Lib;

    async init({ rpId, rpName, timeout }): Promise<void>
    {
        this.f2l = new Fido2Lib({
            timeout,
            rpId,
            rpName,
            challengeSize                  : 128,
            attestation                    : 'none',
            cryptoParams                   : [-8, -7, -257],
            authenticatorAttachment        : 'platform',
            authenticatorRequireResidentKey: true,
            authenticatorUserVerification  : 'preferred',
        });
    }

    async registration({ username, displayName, id })
    {
        const registrationOptions = await this.f2l.attestationOptions();
        const user                = {
            id         : id,
            name       : username,
            displayName: displayName,
        };
        const challenge           = base64.fromArrayBuffer(registrationOptions.challenge, true);

        return {
            ...registrationOptions,
            user,
            status: 'ok',
            challenge,
        };
    }

    async attestation({ clientAttestationResponse, origin, challenge })
    {
        const attestationExpectations = {
            challenge: challenge,
            origin   : origin,
            factor   : 'either' as Factor,
        };

        return await this.f2l.attestationResult(clientAttestationResponse, attestationExpectations);
    }

    async login()
    {
        const assertionOptions = await this.f2l.assertionOptions();
        const challenge        = base64.fromArrayBuffer(assertionOptions.challenge, true);

        return {
            ...assertionOptions,
            attestation: 'direct',
            challenge,
            status     : 'ok',
        };
    }
}
