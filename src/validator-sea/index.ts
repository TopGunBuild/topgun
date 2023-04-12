import Route from 'route-parser';
import {
    PERMISSIVE_SCHEMA as TOPGUN_PERMISSIVE_SCHEMA,
    initAjv as ajvBaseInit,
    createValidator as createBaseValidator,
} from '../validator';
import { SchemaValidateFunction, ValidateFunction } from 'ajv';

const MAX_AUTHOR_ALIAS_SIZE = 512;
const MAX_AUTHOR_ID_SIZE = 128; // ???
const authorPattern = '~:authorId';
const seaAuthorRoute = new Route(authorPattern);
const seaSoulRoute = new Route('*stuff~:authorId.');

export const AUTH_SCHEMA = {
    seaAlias: { type: 'string', maxLength: MAX_AUTHOR_ALIAS_SIZE },
    SEAAlias: {
        type: 'object',
        title: 'TopGun SEA Alias',
        $async: true,
        soul: {
            pattern: '~@:alias',
            properties: {
                alias: { $ref: 'schema.json#/definitions/seaAlias' },
            },
            required: ['alias'],
        },
        additionalProperties: {
            edgeMatchesKey: true,
            anyOf: [{ $ref: '#/definitions/SEAAuthorEdge' }],
        },
    },
    seaAuthorId: { type: 'string', maxLength: MAX_AUTHOR_ID_SIZE },
    seaAuthObj: {
        oneOf: [
            {
                type: 'object',
                properties: {
                    ek: {
                        type: 'object',
                        properties: {
                            ct: { type: 'string' },
                            iv: { type: 'string' },
                            s: { type: 'string' },
                        },
                    },
                    s: { type: 'string' },
                },
            },
            {
                type: 'string',
            },
        ],
    },
    SEAAuthor: {
        type: 'object',
        title: 'TopGun SEA Author',
        $async: true,
        properties: {
            pub: { $ref: '#/definitions/seaAuthorId' },
            epub: { sea: { type: 'string' } },
            alias: { sea: { $ref: 'schema.json#/definitions/seaAlias' } },
            auth: {
                sea: { $ref: 'schema.json#/definitions/seaAuthObj' },
            },
        },
        additionalProperties: {
            sea: {
                anyOf: [
                    { $ref: 'schema.json#/definitions/TopGunEdge' },
                    { $ref: 'schema.json#/definitions/seaAuthObj' },
                    { type: 'null' },
                    { type: 'string' },
                    { type: 'number' },
                    { type: 'boolean' },
                ],
            },
        },
        soul: {
            pattern: authorPattern,
            properties: {
                authorId: { $ref: 'schema.json#/definitions/seaAuthorId' },
            },
            required: ['authorId'],
        },
    },
};

export const PERMISSIVE_NODE_SCHEMA = {
    type: 'object',
    title: 'TopGun SEA Node',
    description: 'Any SEA node supported by TopGun',
    $async: true,

    soul: {
        pattern: '*path~:authorId.',
        properties: {
            path: { type: 'string' },
            authorId: { $ref: 'schema.json#/definitions/seaAuthorId' },
        },
        required: ['path', 'authorId'],
    },
    // additionalProperties: {
    //     '.*': {
    //         sea: {
    //             anyOf: [
    //                 { $ref: 'schema.json#/definitions/TopGunNodeMeta' },
    //                 { $ref: 'schema.json#/definitions/TopGunEdge' },
    //                 { type: 'null' },
    //                 { type: 'string' },
    //                 { type: 'number' },
    //                 { type: 'boolean' }
    //             ]
    //         }
    //     }
    // },
};

export const PERMISSIVE_SCHEMA = {
    ...AUTH_SCHEMA,
    SEANode: PERMISSIVE_NODE_SCHEMA,
    ...TOPGUN_PERMISSIVE_SCHEMA,
};

export const read = (
    data: any,
    key: string,
    pair: boolean | string = false,
) => 
{
    console.log('read', { data, key, pair });
    return Promise.resolve();
    // const packed = pack(data[key], key, data, R.path(['_', '#'], data));

    // return verify(packed, pair as string).then((r: any) =>
    // {
    //     if (typeof r === 'undefined')
    //     {
    //         throw new Error('invalid sea data')
    //     }
    //     return unpack(r, key, data)
    // })
};

const validateSeaProperty =
    (ajv: any) =>
    (
        schema: any,
        data: any,
        pSchema: any,
        _cPath: any,
        parentData: any,
        keyInParent: string,
    ) => 
{
        const soul: string =
            (parentData && parentData['_'] && parentData['_']['#']) || '';

        if (keyInParent === '_') 
{
            return true;
        }
        const obj =
            seaSoulRoute.match(soul) || seaAuthorRoute.match(soul) || {};
        const authorId: string = obj['authorId'] || '';

        if (!authorId) 
{
            return false;
        }
        if (soul === `~${authorId}` && keyInParent === 'pub') 
{
            return data === authorId;
        }

        // Validate as an object to give property validators more context
        const validate = ajv.compile({
            additionalProperties: true,
            properties: {
                [keyInParent]: schema,
            },
        });
        let result: any;

        return read(parentData, keyInParent, authorId)
            .then((res: any) => (result = res))
            .then((res: any) => ({ ...parentData, [keyInParent]: res }))
            .catch((err: any) => 
{
                console.error(
                    'key err',
                    soul,
                    keyInParent,
                    authorId,
                    parentData[keyInParent],
                    err.stack || err,
                );
                return false;
            })
            .then((res: any) => 
{
                if (!res || typeof res[keyInParent] === 'undefined') 
{
                    delete parentData[keyInParent];
                    if (parentData && parentData['_'] && parentData['_']['>']) 
{
                        delete parentData['_']['>'];
                    }
                    console.error(
                        'sea prop err',
                        soul,
                        keyInParent,
                        result,
                        pSchema,
                    );
                    return res;
                }
                return Promise.resolve(validate(res)).then((isValid) => 
{
                    if (!isValid) 
{
                        console.error(
                            'sea validation err',
                            soul,
                            keyInParent,
                            result,
                            validate.errors,
                            pSchema,
                        );
                    }
                    return isValid;
                });
            });
    };

export const initAjv = (conf?: any) => 
{
    const ajv = ajvBaseInit(conf);
    ajv.addKeyword({
        keyword: 'sea',
        async: true,
        modifying: true,
        validate: validateSeaProperty(ajv) as SchemaValidateFunction,
    });
    // ajv.addKeyword("thingHashMatchesSoul", {
    //     validate: thingHashMatchesSoul
    // });
    return ajv;
};

export const createValidator = (): {
    schema: any;
    validate: ValidateFunction<any>;
} => 
{
    return createBaseValidator({
        definitions: PERMISSIVE_SCHEMA,
        init: initAjv,
    });
};
