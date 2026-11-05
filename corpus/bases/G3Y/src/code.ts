import { randomInt } from "node:crypto";

export const CODE_LENGTH = 6;
const ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxy0123456789";
const CODE_PATTERN = new RegExp(`^[A-Za-z0-9]{${CODE_LENGTH}}$`);

export function generateCode(): string {
  let code = "";
  for (let i = 0; i < CODE_LENGTH; i++) {
    code += ALPHABET[randomInt(ALPHABET.length)];
  }
  return code;
}

export function isValidCode(code: string): boolean {
  return CODE_PATTERN.test(code);
}
