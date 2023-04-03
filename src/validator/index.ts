import Route from 'route-parser';
import Ajv, { ValidateFunction } from 'ajv';
import RegexpVisitor from 'route-parser/lib/route/visitors/regexp';
import { isObject } from '../utils/is-object';
import { mergeDeep } from '../utils/merge-deep';
import { DataValidationCxt } from 'ajv/dist/types';

const refRoute = new Route('#/definitions/:refName');

function routeToRegexStr(route: any): string
{
    const { re } = RegexpVisitor.visit(route.ast);
    const reStr  = re.toString();

    return reStr.slice(1, reStr.length - 1);
}

export const PERMISSIVE_SCHEMA = {
    Node: {
        type                : 'object',
        title               : 'Node',
        description         : 'Any node supported by TopGun',
        $async              : true,
        additionalProperties: {
            anyOf: [
                { $ref: '#/definitions/TopGunEdge' },
                { type: 'null' },
                { type: 'string' },
                { type: 'number' },
                { type: 'boolean' },
                { type: 'object' }
            ]
        },
        soul                : {
            pattern   : '*soul',
            type      : 'object',
            properties: {
                soul: { type: 'string' }
            },
            required  : ['soul']
        }
    }
};

const DEFAULT_SCHEMA = PERMISSIVE_SCHEMA;

const compileValidateSoul = (ajv: any) => (schema: any) =>
{
    schema = schema || {};

    const matchSchema = { ...schema };
    delete matchSchema['pattern'];

    const pattern: string = schema['pattern'] || '';
    const route           = new Route(pattern);

    return (data: any, dataCxt?: DataValidationCxt): boolean =>
    {
        const soul = data && data['_'] && data['_']['#'];

        if (!soul || !pattern || soul !== dataCxt.parentDataProperty)
        {
            return false;
        }
        const match = route.match(soul);

        return match ? ajv.compile(matchSchema)(match) : false;
    }
};

const compilePropsFromSoul = (propMap: any, parentSchema: any) =>
{
    const pattern: string = parentSchema && parentSchema['soul'] && parentSchema['soul']['pattern'] || '';
    const route           = new Route(pattern);

    return (data: any) =>
    {
        const soul: string = data && data['_'] && data['_']['#'] || '';
        const soulProps    = route.match(soul) || {};

        return !Object
            .keys(propMap)
            .find(propName =>
            {
                if (!(propName in data))
                {
                    return false;
                }
                return soulProps[propName] !== data[propMap[propName]]
            });
    }
};

const compileEdgeMatchesKey = (schema: any) => (
    data: any,
    _cPath: any,
    _parentData?: any,
    keyInParent?: any
) => (schema ? data['#'] === keyInParent : true);

export function initAjv({ coerceTypes = true, removeAdditional = false, ...config } = {})
{
    const ajv = new Ajv({ coerceTypes, removeAdditional, ...config });

    ajv.addKeyword({
        keyword: 'soul',
        compile: compileValidateSoul(ajv),
    });
    ajv.addKeyword({
        keyword: 'edgeMatchesKey',
        compile: compileEdgeMatchesKey
    });
    ajv.addKeyword({
        keyword: 'propsFromSoul',
        compile: compilePropsFromSoul
    });
    return ajv
}

export function createValidator({
                                    init = initAjv,
                                    id = 'http://example.com/schemas/topgun-schema.json',
                                    jsonSchema = 'http://json-schema.org/draft-07/schema#',
                                    title = 'TopGun Message Schema',
                                    description = 'A defintion for the TopGun wire protocol',
                                    definitions: supplied = DEFAULT_SCHEMA
                                } = {}): {schema: any, validate: ValidateFunction<any>}
{
    const nodeTypes: string[] = [];
    const definitions         = Object
        .keys(supplied)
        .reduce((defs: any, typeName) =>
        {
            const pattern = defs[typeName]?.soul?.pattern || '';

            if (!pattern)
            {
                return defs;
            }
            const route     = new Route(pattern);
            const pathOrRef = (p: string[]) =>
            {
                const val             = p.reduce((accum, path) => accum && accum[path], defs);
                const ref: string     = val && val['$refs'];
                const refRouteParams  = refRoute.match(ref || '');
                const refName: string = refRouteParams && refRouteParams['refName'];
                const result          = !!refName ? defs[refName] : val;

                return isObject(result) ? result : {};
            };

            nodeTypes.push(typeName);

            return mergeDeep(
                {},
                defs,
                {
                    [typeName]: {
                        '$async'  : true,
                        required  : [
                            '_',
                            ...(defs && defs[typeName] && defs[typeName]['required'] || [])
                        ],
                        type      : 'object',
                        properties: {
                            '_': {
                                type      : 'object',
                                allOf     : [{ $ref: '#/definitions/TopGunNodeMeta' }],
                                properties: {
                                    '#': { $ref: `#/definitions/${typeName}Soul` },
                                    '>': {
                                        type             : 'object',
                                        properties       : Object
                                            .keys(pathOrRef([typeName, 'properties']))
                                            .reduce(
                                                (props, key) => ({ ...props, [key]: { type: 'number' } }),
                                                {}
                                            ),
                                        patternProperties: Object
                                            .keys(pathOrRef([typeName, 'patternProperties']))
                                            .reduce(
                                                (props, key) => ({ ...props, [key]: { type: 'number' } }),
                                                {}
                                            )
                                    }
                                }
                            }
                        }
                    }
                },
                {
                    [`${typeName}Soul`]: {
                        type   : 'string',
                        pattern: routeToRegexStr(route)
                    }
                },
                {
                    [`${typeName}Edge`]: {
                        type                : 'object',
                        additionalProperties: false,
                        properties          : {
                            '#': { $ref: `#/definitions/${typeName}Soul` }
                        },
                        required            : ['#']
                    }
                }
            );
        }, supplied);

    const schema = {
        $id        : id,
        $schema    : jsonSchema,
        $async     : true,
        title,
        description,
        anyOf      : [{ $ref: '#/definitions/TopGunMsg' }],
        definitions: {
            TopGunMsg         : {
                $async              : true,
                type                : 'object',
                required            : ['#'], // necessary over wire
                additionalProperties: false,
                properties          : {
                    '#'  : {
                        title      : 'Message Identifier',
                        description: 'This should be a globally unique identifier',
                        type       : 'string'
                    },
                    '##' : {
                        title      : 'Fast Hash Value?',
                        description: 'I have no idea how this is calculated',
                        type       : 'number'
                    },
                    '@'  : {
                        title      : 'Responding To',
                        description: 'The message identifier this message is responding to',
                        type       : 'string'
                    },
                    '><' : {
                        title      : 'Adjacent Peers',
                        description: 'Not really sure how this works',
                        type       : 'string'
                    },
                    $    : {
                        title: '??'
                    },
                    I    : {
                        title: '??'
                    },
                    ok   : {
                        title      : '??',
                        description: 'Shouldn\'t actually be sent over wire',
                        type       : 'boolean'
                    },
                    how  : {
                        title      : 'Used for debugging',
                        description: 'Shouldn\'t actually be sent over wire (but it is)',
                        type       : 'string'
                    },
                    mesh : {
                        title      : '??',
                        description: 'Shouldn\'t be sent over wire'
                    },
                    rad  : {
                        title      : '??',
                        description: 'Shouldn\'t be sent over wire'
                    },
                    user : {
                        title      : '??',
                        description: 'I don\'t think this is supposed to be sent over wire'
                    },
                    err  : {
                        anyOf: [{ type: 'null' }, { type: 'string' }]
                    },
                    leech: {
                        title      : 'Leech Command',
                        description: 'TopGun protocol extension added by pistol',
                        type       : 'boolean'
                    },
                    ping : {
                        title      : 'Ping Command',
                        description: 'TopGun protocol extension added by pistol',
                        type       : 'boolean'
                    },
                    get  : {
                        title               : 'Get Command',
                        description         : 'A request for graph data',
                        type                : 'object',
                        additionalProperties: false,
                        properties          : {
                            '#': {
                                description: 'The soul to request data for',
                                anyOf      : nodeTypes.map(name => ({
                                    $ref: `#/definitions/${name}Soul`
                                }))
                            },
                            '.': {
                                description: 'Request a single property?',
                                type       : 'string'
                            }
                        }
                    },
                    put  : {
                        anyOf: [
                            {
                                $async              : true,
                                title               : 'Put Command',
                                description         : 'A payload of graph data',
                                type                : 'object',
                                additionalProperties: {
                                    anyOf: [
                                        ...nodeTypes.map(name => ({
                                            $ref: `#/definitions/${name}`
                                        })),
                                        { type: 'null' }
                                    ]
                                }
                            },
                            { type: 'null' }
                        ]
                    }
                }
            },
            TopGunChangeStates: {
                type             : 'object',
                title            : 'TopGun Change States',
                description      : 'A map of property names to update timestamps',
                patternProperties: {
                    '.*': {
                        type: 'number'
                    }
                }
            },
            TopGunNodeMeta: {
                title               : 'TopGun Node Metadata',
                description         : 'Change State and soul of a node',
                type                : 'object',
                additionalProperties: false,
                properties          : {
                    '#': { title: 'Soul', type: 'string' },
                    '>': { $ref: '#/definitions/TopGunChangeStates' }
                },
                required            : ['#', '>']
            },
            TopGunEdge    : {
                type                : 'object',
                additionalProperties: false,
                properties          : {
                    '#': { type: 'string' }
                },
                required            : ['#']
            },
            ...definitions
        }
    };
    const ajv    = init();

    ajv.addSchema({
        $id        : 'schema.json',
        definitions: schema.definitions
    });
    return { schema, validate: ajv.compile(schema) }
}
