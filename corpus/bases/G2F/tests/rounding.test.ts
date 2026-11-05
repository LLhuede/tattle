import { describe, expect, it } from "vitest";
import { splitEvenly } from "../src/settle.js";

describe("splitEvenly rounding", () => {
  it("gives spare cents to the last participants", () => {
    expect(splitEvenly(1000, 3)).toEqual([333, 333, 334]);
  });
});
