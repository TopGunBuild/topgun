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

export interface TGConfigLoggerType {
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
