export interface TGLoggerTransportOptions
{
    msg: any;
    rawMsg: any;
    level: string;
    extension?: string|null;
    options?: any;
}

export type TGLoggerTransportFunctionType = (props: TGLoggerTransportOptions) => any;

export interface TGExtendedLoggerType
{
    debug?(...args: any[]): void;

    log?(...args: any[]): void;

    warn?(...args: any[]): void;

    error?(...args: any[]): void;
}

export type TGLoggerLevel = 'debug' | 'log' | 'warn' | 'error';

export interface TGLoggerOptions
{
    appName?: string;
    appId?: string|number;
    transport?: TGLoggerTransportFunctionType|TGLoggerTransportFunctionType[];
    transportOptions?: any;
    levels?: TGLoggerLevel[];
    async?: boolean;
    asyncFunc?: (...args: any[]) => any;
    stringifyFunc?: (msg: any) => string;
    dateFormat?: string|((date: Date) => string); //"time" | "local" | "utc" | "iso" | "function";
    printLevel?: boolean;
    printDate?: boolean;
    enabled?: boolean;
    enabledExtensions?: string[]|string|null;
}

export interface TGLoggerType extends TGExtendedLoggerType
{
    extend(extension: string): TGExtendedLoggerType;

    enable(extension?: string): boolean;

    disable(extension?: string): boolean;

    getExtensions(): string[];

    patchConsole(): void;
}
