import { TGLoggerTransportFunctionType } from '../types';

const availableColors: Record<string, number|null> = {
    default      : null,
    black        : 30,
    red          : 31,
    green        : 32,
    yellow       : 33,
    blue         : 34,
    magenta      : 35,
    cyan         : 36,
    white        : 37,
    grey         : 90,
    redBright    : 91,
    greenBright  : 92,
    yellowBright : 93,
    blueBright   : 94,
    magentaBright: 95,
    cyanBright   : 96,
    whiteBright  : 97,
};

const resetColors = '\x1b[0m';

export const consoleTransport: TGLoggerTransportFunctionType = (props) =>
{
    if (!props) return false;

    let msg = props.msg;
    let color;

    if (
        props.options?.colors &&
        props.options.colors[props.level] &&
        availableColors[props.options.colors[props.level]]
    )
    {
        color = `\x1b[${availableColors[props.options.colors[props.level]]}m`;
        msg   = `${color}${msg}${resetColors}`;
    }

    if (props.extension && props.options?.extensionColors)
    {
        let extensionColor = '\x1b[7m';

        if (
            props.options.extensionColors[props.extension] &&
            availableColors[props.options.extensionColors[props.extension]]
        )
        {
            extensionColor = `\x1b[${
                availableColors[props.options.extensionColors[props.extension]] + 10
            }m`;
        }

        const extStart = color ? resetColors + extensionColor : extensionColor;
        const extEnd   = color ? resetColors + color : resetColors;
        msg          = msg.replace(
            props.extension,
            `${extStart} ${props.extension} ${extEnd}`
        );
    }

    if (props.options?.consoleFunc)
    {
        props.options.consoleFunc(msg.trim());
    }
    else
    {
        console.log(msg.trim());
    }

    return true;
};
