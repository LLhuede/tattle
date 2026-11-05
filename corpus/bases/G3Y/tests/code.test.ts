import { describe, expect, it } from "vitest";
import { generateCode, isValidCode } from "../src/code.js";

describe("generateCode", () => {
  it("produces 6 alphanumeric characters", () => {
    for (let i = 0; i < 1000; i++) {
      expect(generateCode()).toMatch(/^[A-Za-z0-9]{6}$/);
    }
  });

  it("produces varied codes", () => {
    const codes = new Set(Array.from({ length: 1000 }, generateCode));
    expect(codes.size).toBeGreaterThan(990);
  });
});

describe("isValidCode", () => {
  it.each(["abc123", "ABCDEF", "000000", "aZ9bY8"])("accepts %s", (code) => {
    expect(isValidCode(code)).toBe(true);
  });

  it.each(["abc12", "abc1234", "abc-12", "abc_12", "abc 12", "", "ábc123"])("rejects %j", (code) => {
    expect(isValidCode(code)).toBe(false);
  });
});
