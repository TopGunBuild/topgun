export type InvalidResult = {
  isValid: false
  error: ValidationError
}

export type ValidResult = {
  isValid: true
}

export class ValidationError extends Error {
  public type: string;
  public details?: unknown

  constructor(message: string, details?: any) {
    super()
    this.message = message
    this.details = details
  }
}

export type ValidationResult = ValidResult | InvalidResult;
