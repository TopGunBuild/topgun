/**
 * Transform a socket URL
 * @param url The URL to transform
 * @returns The transformed URL
 */
export function transformSocketUrl(url: string): string {
    return url.replace(/^https?:\/\//, (match) => match === 'https://' ? 'wss://' : 'ws://');
}
