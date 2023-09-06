export function removeProtocolFromUrl(url: string): string
{
    return url.replace(/(^\w+:|^)\/\//, '');
}