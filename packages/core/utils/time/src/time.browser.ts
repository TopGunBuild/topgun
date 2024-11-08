function nowBig()
{
    const now = window.performance.now();
    return BigInt(now | 0) * n1e6 + BigInt((now % 1 * 1e6) | 0);
}

const n1e6      = BigInt(1e6);
const startTime = BigInt(Date.now()) * n1e6 - nowBig();

export const bigintTime = (): bigint => startTime + nowBig();
