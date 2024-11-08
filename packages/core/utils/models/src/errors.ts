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

export class DecryptionError extends Error {
    constructor(message: string) {
        super(message)
        this.name = 'DecryptionError'
    }
}

export class EncryptionError extends Error {
    constructor(message: string) {
        super(message)
        this.name = 'EncryptionError'
    }
} 