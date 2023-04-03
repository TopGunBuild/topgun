import { Buffer } from './shims';

export const base64: any = {};

if (typeof btoa === 'undefined')
{
    base64.btoa = function btoa(b: any): string
    {
        return Buffer.from(b, 'binary').toString('base64')
    }
}
else
{
    base64.btoa = (x: string) => btoa(x)
}

if (typeof atob === 'undefined')
{
    base64.atob = function atob(b: any): string
    {
        return Buffer.from(b, 'base64').toString('binary');
    }
}
else
{
    base64.atob = (x: string) => atob(x)
}
