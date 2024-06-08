const start = BigInt(Date.now()) * BigInt(1e6) - process.hrtime.bigint()

export const bigintTime = (): bigint => start + process.hrtime.bigint()
