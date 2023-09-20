import {
    TGLoggerOptions,
    TGExtendedLogType,
    TGLoggerLevelsType,
    TGLoggerTransportFunctionType,
    TGLoggerType
} from './types';
import { defaultLogger, reservedKey } from './constants';

export class TGLogger implements TGLoggerType
{
    private readonly _levels: TGLoggerLevelsType;
    private readonly _transport: TGLoggerTransportFunctionType|TGLoggerTransportFunctionType[];
    private readonly _transportOptions: any;
    private readonly _async: boolean;
    private readonly _asyncFunc: (...args: any[]) => any;
    private readonly _stringifyFunc: (msg: any) => string;
    private readonly _dateFormat: string|((date: Date) => string);
    private readonly _printLevel: boolean;
    private readonly _printDate: boolean;
    private _level: string;
    private _enabled: boolean;
    private _enabledExtensions: string[]|null                 = null;
    private _extensions: string[]                             = [];
    private _extendedLogs: {[key: string]: TGExtendedLogType} = {};
    private _originalConsole?: typeof console;

    /**
     * Constructor
     */
    constructor(config: TGLoggerOptions)
    {
        this._levels = config.levels;
        this._level  = config.severity || Object.keys(this._levels)[0];

        this._transport        = config.transport;
        this._transportOptions = config.transportOptions;

        this._asyncFunc = config.asyncFunc;
        this._async     = config.async;

        this._stringifyFunc = config.stringifyFunc;

        this._dateFormat = config.dateFormat;

        this._printLevel = config.printLevel;
        this._printDate  = config.printDate;

        this._enabled = config.enabled;

        if (Array.isArray(config.enabledExtensions))
        {
            this._enabledExtensions = config.enabledExtensions;
        }
        else if (typeof config.enabledExtensions === 'string')
        {
            this._enabledExtensions = [config.enabledExtensions];
        }

        /** Bind correct log levels methods */
        /* eslint-disable-next-line @typescript-eslint/no-this-alias */
        const _this: any = this;
        Object.keys(this._levels).forEach((level: string) =>
        {
            if (typeof level !== 'string')
            {
                throw Error('[topgun-logs] ERROR: levels must be strings');
            }
            if (level[0] === '_')
            {
                throw Error(
                    '[topgun-logs] ERROR: keys with first char "_" is reserved and cannot be used as levels'
                );
            }
            if (reservedKey.indexOf(level) !== -1)
            {
                throw Error(
                    `[topgun-logs] ERROR: [${level}] is a reserved key, you cannot set it as custom level`
                );
            }
            if (typeof this._levels[level] === 'number')
            {
                _this[level] = this.#log.bind(this, level, null);
            }
            else
            {
                throw Error(`[topgun-logs] ERROR: [${level}] wrong level config`);
            }
        }, this);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Extend logger with a new extension
     */
    extend(extension: string): TGExtendedLogType
    {
        if (extension === 'console')
        {
            throw Error(
                '[topgun-logs:extend] ERROR: you cannot set [console] as extension, use patchConsole instead'
            );
        }
        if (this._extensions.includes(extension))
        {
            return this._extendedLogs[extension];
        }
        this._extendedLogs[extension] = {};
        this._extensions.push(extension);
        const extendedLog = this._extendedLogs[extension];
        Object.keys(this._levels).forEach((level: string) =>
        {
            extendedLog[level]                = (...msgs: any) =>
            {
                this.#log(level, extension, ...msgs);
            };
            extendedLog['extend']             = (extension: string) =>
            {
                throw Error(
                    '[topgun-logs] ERROR: you cannot extend a logger from an already extended logger'
                );
            };
            extendedLog['enable']             = () =>
            {
                throw Error(
                    '[topgun-logs] ERROR: You cannot enable a logger from extended logger'
                );
            };
            extendedLog['disable']            = () =>
            {
                throw Error(
                    '[topgun-logs] ERROR: You cannot disable a logger from extended logger'
                );
            };
            extendedLog['getExtensions']      = () =>
            {
                throw Error(
                    '[topgun-logs] ERROR: You cannot get extensions from extended logger'
                );
            };
            extendedLog['setSeverity']        = (level: string) =>
            {
                throw Error(
                    '[topgun-logs] ERROR: You cannot set severity from extended logger'
                );
            };
            extendedLog['getSeverity']        = () =>
            {
                throw Error(
                    '[topgun-logs] ERROR: You cannot get severity from extended logger'
                );
            };
            extendedLog['patchConsole']       = () =>
            {
                throw Error(
                    '[topgun-logs] ERROR: You cannot patch console from extended logger'
                );
            };
            extendedLog['getOriginalConsole'] = () =>
            {
                throw Error(
                    '[topgun-logs] ERROR: You cannot get original console from extended logger'
                );
            };
        });
        return extendedLog;
    };

    /**
     * Enable logger or extension
     */
    enable(extension?: string): boolean
    {
        if (!extension)
        {
            this._enabled = true;
            return true;
        }

        if (this._extensions.includes(extension))
        {
            if (this._enabledExtensions)
            {
                if (!this._enabledExtensions.includes(extension))
                {
                    this._enabledExtensions.push(extension);
                    return true;
                }
                else
                {
                    return true;
                }
            }
            else
            {
                this._enabledExtensions = [];
                this._enabledExtensions.push(extension);
                return true;
            }
        }
        else
        {
            throw Error(
                `[topgun-logs:enable] ERROR: Extension [${extension}] not exist`
            );
        }
    };

    /**
     * Disable logger or extension
     */
    disable(extension?: string): boolean
    {
        if (!extension)
        {
            this._enabled = false;
            return true;
        }
        if (this._extensions.includes(extension))
        {
            if (this._enabledExtensions)
            {
                const extIndex = this._enabledExtensions.indexOf(extension);
                if (extIndex > -1)
                {
                    this._enabledExtensions.splice(extIndex, 1);
                }
                return true;
            }
            else
            {
                return true;
            }
        }
        else
        {
            throw Error(
                `[topgun-logs:disable] ERROR: Extension [${extension}] not exist`
            );
        }
    }

    /**
     * Return all created extensions
     */
    getExtensions(): string[]
    {
        return this._extensions;
    }

    /**
     * Set log severity API
     */
    setSeverity(level: string): string
    {
        if (level in this._levels)
        {
            this._level = level;
        }
        else
        {
            throw Error(
                `[topgun-logs:setSeverity] ERROR: Level [${level}] not exist`
            );
        }
        return this._level;
    }

    /**
     * Get current log severity API
     */
    getSeverity(): string
    {
        return this._level;
    }

    /**
     * Monkey Patch global console.log
     */
    patchConsole(): void
    {
        const extension = 'console';
        const levelKeys = Object.keys(this._levels);

        if (!this._originalConsole)
        {
            this._originalConsole = console;
        }

        if (!this._transportOptions.consoleFunc)
        {
            this._transportOptions.consoleFunc = this._originalConsole.log;
        }

        console['log'] = (...msgs: any) =>
        {
            this.#log(levelKeys[0], extension, ...msgs);
        };

        levelKeys.forEach((level: string) =>
        {
            if ((console as any)[level])
            {
                (console as any)[level] = (...msgs: any) =>
                {
                    this.#log(level, extension, ...msgs);
                };
            }
            else
            {
                this._originalConsole &&
                this._originalConsole.log(
                    `[topgun-logs:patchConsole] WARNING: "${level}" method does not exist in console and will not be available`
                );
            }
        });
    };

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Log messages methods and level filter
     */
    #log(level: string, extension: string|null, ...msgs: any[]): boolean
    {
        if (this._async)
        {
            return this._asyncFunc(() =>
            {
                this.#sendToTransport(level, extension, msgs);
            });
        }
        else
        {
            return this.#sendToTransport(level, extension, msgs);
        }
    }

    #sendToTransport(level: string, extension: string|null, msgs: any): boolean
    {
        if (!this._enabled) return false;
        if (!this.#isLevelEnabled(level))
        {
            return false;
        }
        if (extension && !this.#isExtensionEnabled(extension))
        {
            return false;
        }
        const msg            = this.#formatMsg(level, extension, msgs);
        const transportProps = {
            msg      : msg,
            rawMsg   : msgs,
            level    : { severity: this._levels[level], text: level },
            extension: extension,
            options  : this._transportOptions,
        };
        if (Array.isArray(this._transport))
        {
            for (let i = 0; i < this._transport.length; i++)
            {
                this._transport[i](transportProps);
            }
        }
        else
        {
            this._transport(transportProps);
        }
        return true;
    }

    #stringifyMsg(msg: any): string
    {
        return this._stringifyFunc(msg);
    }

    #formatMsg(level: string, extension: string|null, msgs: any): string
    {
        let nameTxt: string = '';
        if (extension)
        {
            nameTxt = `${extension} | `;
        }

        let dateTxt: string = '';
        if (this._printDate)
        {
            if (typeof this._dateFormat === 'string')
            {
                switch (this._dateFormat)
                {
                case 'time':
                    dateTxt = `${new Date().toLocaleTimeString()} | `;
                    break;
                case 'local':
                    dateTxt = `${new Date().toLocaleString()} | `;
                    break;
                case 'utc':
                    dateTxt = `${new Date().toUTCString()} | `;
                    break;
                case 'iso':
                    dateTxt = `${new Date().toISOString()} | `;
                    break;
                default:
                    break;
                }
            }
            else if (typeof this._dateFormat === 'function')
            {
                dateTxt = this._dateFormat(new Date());
            }
        }

        let levelTxt = '';
        if (this._printLevel)
        {
            levelTxt = `${level.toUpperCase()} : `;
        }

        let stringMsg: string = dateTxt + nameTxt + levelTxt;

        if (Array.isArray(msgs))
        {
            for (let i = 0; i < msgs.length; ++i)
            {
                stringMsg += this.#stringifyMsg(msgs[i]);
            }
        }
        else
        {
            stringMsg += this.#stringifyMsg(msgs);
        }

        return stringMsg;
    };

    /**
     * Return true if level is enabled
     */
    #isLevelEnabled(level: string): boolean
    {
        return this._levels[level] >= this._levels[this._level];
    };

    /**
     * Return true if extension is enabled
     */
    #isExtensionEnabled(extension: string): boolean
    {
        if (!this._enabledExtensions)
        {
            return true;
        }

        return this._enabledExtensions.includes(extension);
    };
}

export const createLogger = <Y extends string>(config?: TGLoggerOptions) =>
{
    type levelMethods<levels extends string> = {
        [key in levels]: (...args: unknown[]) => void;
    };

    type loggerType = levelMethods<Y>;

    type extendMethods = {
        extend: (extension: string) => loggerType;
    };

    const mergedConfig = { ...defaultLogger, ...config };

    return new TGLogger(mergedConfig) as unknown as Omit<TGLogger, 'extend'>&
    loggerType&
    extendMethods;
};
