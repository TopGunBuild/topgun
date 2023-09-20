export interface TGLoggerTransportOptions {
    msg: any;
    rawMsg: any;
    level: {severity: number; text: string};
    extension?: string|null;
    options?: any;
}

export type TGLoggerTransportFunctionType = (props: TGLoggerTransportOptions) => any;

export type TGLoggerLevelsType = {[key: string]: number};

export type TGLevelLogMethodType = (...msgs: any[]) => boolean;

export type TGExtendedLogType = {[key: string]: TGLevelLogMethodType|any};

export interface TGLoggerOptions {
    severity?: string;
    transport?: TGLoggerTransportFunctionType|TGLoggerTransportFunctionType[];
    transportOptions?: any;
    levels?: TGLoggerLevelsType;
    async?: boolean;
    asyncFunc?: (...args: any[]) => any;
    stringifyFunc?: (msg: any) => string;
    dateFormat?: string|((date: Date) => string); //"time" | "local" | "utc" | "iso" | "function";
    printLevel?: boolean;
    printDate?: boolean;
    enabled?: boolean;
    enabledExtensions?: string[]|string|null;
}

export interface TGLoggerType
{
    extend(extension: string): TGExtendedLogType;
    enable(extension?: string): boolean;
    disable(extension?: string): boolean;
    getExtensions(): string[];
    setSeverity(level: string): string;
    getSeverity(): string;
    patchConsole(): void;
    debug?(...args: any[]): void;
    info?(...args: any[]): void;
    warn?(...args: any[]): void;
    error?(...args: any[]): void;

    [key: string]: any;
}