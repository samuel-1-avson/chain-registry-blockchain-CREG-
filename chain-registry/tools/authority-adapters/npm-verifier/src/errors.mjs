export class Rejected extends Error {
  constructor(message, code = "ERR_NPM_AUTHORITY_REJECTED", options) {
    super(message, options);
    this.name = "Rejected";
    this.code = code;
  }
}

export function reject(message, code, options) {
  throw new Rejected(message, code, options);
}
