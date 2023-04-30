import {
    isObject,
    Struct,
    Shape,
    number,
    object,
    isOk,
    ok,
    union,
    string,
    boolean,
    record,
    nullable,
    err,
    StructError,
    isString,
} from 'topgun-typed';

const attributeTimestampStruct: Struct<any> = (node) => 
{
    const shape: Shape = Object.keys(node)
        .filter(key => key !== '_')
        .reduce((accum, key) => ({ ...accum, [key]: number() }), {});

    const struct = object(shape);
    const result = struct(node._['>']);

    return isOk(result) ? ok(node) : result;
};

const nodeTypesStruct: Struct<any> = (node) => 
{
    const nodeWithoutState = { ...node };
    delete nodeWithoutState['_'];

    // Node values must any of: null, string, number, boolean, object with '#' reference
    const valueStruct = union(
        [number(), string(), boolean(), object({ '#': string() })],
        'Node value must be null string, number, boolean or object.',
    );
    const struct = record(string(), nullable(valueStruct));
    const result = struct(nodeWithoutState);

    return isOk(result) ? attributeTimestampStruct(node) : result;
};

const nodeStruct =
    (graph): Struct<any> =>
        (input) => 
        {
            if (!isObject(input)) 
            {
                return err(
                    new StructError('Node must be object', { input, path: [] }),
                );
            }
            if (!isObject(input._)) 
            {
                return err(
                    new StructError('Node state must be object in path _', {
                        input,
                        path: ['_'],
                    }),
                );
            }
            if (!isObject(input._['>'])) 
            {
                return err(
                    new StructError('Node state must be object in path _.[\'>\']', {
                        input,
                        path: ['_', '>'],
                    }),
                );
            }
            if (!isString(input._['#'])) 
            {
                return err(
                    new StructError('Soul must be string in _.[\'#\']', {
                        input,
                        path: ['_', '#'],
                    }),
                );
            }

            const soul = input._['#'];

            if (!isObject(graph[soul])) 
            {
                return err(
                    new StructError('Soul must be present in root graph', {
                        input,
                        path: [],
                    }),
                );
            }

            return nodeTypesStruct(input);
        };

export const validator: Struct<any> = (graph) => 
{
    if (!isObject(graph)) 
    {
        return err(
            new StructError('Root graph must be object', {
                input: graph,
                path: [],
            }),
        );
    }

    const struct = record(string(), nodeStruct(graph));
    const result = struct(graph);

    return isOk(result) ? ok(graph) : result;
};
