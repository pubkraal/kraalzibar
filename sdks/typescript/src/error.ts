export type ErrorCode =
  | "INVALID_ARGUMENT"
  | "NOT_FOUND"
  | "UNAUTHENTICATED"
  | "FAILED_PRECONDITION"
  | "RESOURCE_EXHAUSTED"
  | "INTERNAL"
  | "UNKNOWN";

export class KraalzibarError extends Error {
  readonly code: ErrorCode;

  constructor(code: ErrorCode, message: string) {
    super(message);
    this.name = "KraalzibarError";
    this.code = code;
  }
}

export const mapHttpStatus = (status: number): ErrorCode => {
  switch (status) {
    case 400:
      return "INVALID_ARGUMENT";
    case 401:
      return "UNAUTHENTICATED";
    case 404:
      return "NOT_FOUND";
    case 412:
      return "FAILED_PRECONDITION";
    case 422:
      return "RESOURCE_EXHAUSTED";
    case 500:
      return "INTERNAL";
    default:
      return "UNKNOWN";
  }
};
