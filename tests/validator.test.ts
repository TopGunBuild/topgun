import { createValidator } from '../src/validator';

describe('validate', () =>
{
    it('graph must be valid', async () =>
    {
        const suppressor = createValidator();
        const graph      = {
            'user': {
                '_'   : {
                    '#': 'user',
                    '>': {
                        'said': 1674579463964
                    }
                },
                'said': {
                    'say': 'Hello'
                },
            },
        };

        try
        {
            const isValid = await suppressor.validate({
                '#': 'dummymsgid',
                put: graph
            });

            expect(isValid).toBeTruthy();
        }
        catch (e)
        {
            expect(e).toMatch('error');
        }
    })
});
