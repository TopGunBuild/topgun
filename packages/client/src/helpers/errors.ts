export class ValidationError extends Error {
    constructor(message: string) {
        super(message)
        this.name = 'ValidationError'
    }
}

export class CryptoError extends Error {
    constructor(message: string) {
        super(message)
        this.name = 'CryptoError'
    }
}