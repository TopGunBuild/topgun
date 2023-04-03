export function pseudoRandomText(
    l = 24,
    c = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXZabcdefghijklmnopqrstuvwxyz'
): string
{
    let s = '';
    while (l > 0)
    {
        s += c.charAt(Math.floor(Math.random() * c.length));
        l--
    }
    return s
}
