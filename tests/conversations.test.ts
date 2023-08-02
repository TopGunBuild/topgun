import { TGMessage, TGUserCredentials } from '../src/types';
import { certify, pair } from '../src/sea';
import { TGClient } from '../src/client';
import { wait } from './test-util';

let client: TGClient;

describe('Conversation', () =>
{
    beforeEach(() =>
    {
        client = new TGClient();
    });
    afterEach(async () =>
    {
        await client.disconnect();
    });

    it('accept a request from a participant ', async () =>
    {
        // Auth as john
        const participant = await client.user().create('john', '12345678');
        // Generate certificate for all participant requests
        await generateParticipantRequestsCertificate();

        // Auth as billy
        const user = await client.user().create('billy', '12345678');

        // Send request to john
        const addParticipantCert = await addRequest(participant.pub, user);

        client.user().leave();
        await wait(10);

        // Auth as john
        await client.user().auth('john', '12345678');

        const requestsStream = client.user()
            .get('participantRequests')
            .map()
            .once<any>();

        let result: TGMessage;
        let request: any;
        let requestKey: string;

        // Get request from billy
        for await (const { value, key } of requestsStream)
        {
            request    = value;
            requestKey = key;

            // Accept request from billy
            result = await acceptRequest(requestKey, request.cert);
            requestsStream.destroy();
        }

        expect(result.ok).toBeTruthy();
    });
});

async function generateAddParticipantCertificate(participantPub: string, userCredentials: TGUserCredentials): Promise<string>
{
    const certificateExists = await client.user()
        .get('certificates')
        .get(participantPub)
        .get('addParticipant')
        .promise<string>();

    if (certificateExists) return;

    const certificate = await certify(
        [participantPub],
        [{ '*': 'participants' }],
        userCredentials,
        null
    );

    await client.user()
        .get('certificates')
        .get(participantPub)
        .get('addParticipant')
        .put(certificate);

    return certificate;
}

async function generateParticipantRequestsCertificate(): Promise<string>
{
    const certificateExists = await client.user()
        .get('certificates')
        .get('participantRequests')
        .promise<string>();

    if (certificateExists) return certificateExists;

    const certificate = await certify(
        ['*'],
        [{ '*': 'participantRequests' }],
        await pair(),
        null
    );

    await client.user()
        .get('certificates')
        .get('participantRequests')
        .put(certificate);

    return certificate;
}

async function addRequest(participantPub: string, userCredentials: TGUserCredentials): Promise<string>
{
    // Get his request certificate
    const addRequestCertificate = await client
        .user(participantPub)
        .get('certificates')
        .get('participantRequests')
        .promise<string>();

    // console.log('c-', addRequestCertificate);

    if (!addRequestCertificate)
    {
        throw Error(`Could not find participant certificate to add request`);
    }

    // console.log(
    //     addRequestCertificate
    // );

    // Save your request to him
    const setRequest = await client
        .user(participantPub)
        .get('participantRequests')
        .set(
            { cert: client.user().is.pub },
            { cert: addRequestCertificate }
        );

    if (setRequest.err)
    {
        throw Error(`Participant request error`);
    }

    return await generateAddParticipantCertificate(participantPub, userCredentials);
}

async function acceptRequest(requestKey: string, participantPub: string): Promise<TGMessage>
{
    // console.log(requestKey);
    // Clear request
    await client.get(requestKey).put(null);
    // console.log('after');
    // await client.user()
    //     .get('participantRequests')
    //     .get(requestKey)
    //     .put(null);

    // await client.user()
    //     .get('participantRequests')
    //     .get(requestKey)
    //     .put(null);

    // Get your certificate
    const addParticipantCertificate = await client
        .user(participantPub)
        .get('certificates')
        .get(client.user().is.pub)
        .get('addParticipant')
        .promise<string>();

    // Add yourself to his participant list
    const setPub = await client
        .user(participantPub)
        .get('participants')
        .set(
            { cert: client.user().is.pub },
            { cert: addParticipantCertificate }
        );

    if (setPub.err)
    {
        throw Error(`Add participant failed`);
    }

    // Add him to your participant list
    return await client.user()
        .get('participants')
        .set({
            cert: participantPub
        });
}