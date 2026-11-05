import request from "supertest";
import { beforeEach, describe, expect, it } from "vitest";
import { createApp } from "../src/app.js";
import { openDb, type Db } from "../src/db.js";

let db: Db;

beforeEach(() => {
  db = openDb(":memory:");
});

describe("POST /api/shorten", () => {
  it("creates a short link", async () => {
    const app = createApp(db, { baseUrl: "https://sho.rt" });
    const res = await request(app).post("/api/shorten").send({ url: "https://example.com/some/page?x=1" });

    expect(res.status).toBe(201);
    expect(res.body.code).toMatch(/^[A-Za-z0-9]{6}$/);
    expect(res.body.shortUrl).toBe(`https://sho.rt/${res.body.code}`);
    expect(res.body.url).toBe("https://example.com/some/page?x=1");
  });

  it("stores the link in sqlite", async () => {
    const app = createApp(db);
    const res = await request(app).post("/api/shorten").send({ url: "https://example.com" });
    const row = db.prepare("SELECT url FROM urls WHERE code = ?").get(res.body.code);
    expect(row).toEqual({ url: "https://example.com" });
  });

  it("uses the request host when no baseUrl is configured", async () => {
    const app = createApp(db);
    const res = await request(app).post("/api/shorten").set("Host", "localhost:1234").send({ url: "https://example.com" });
    expect(res.body.shortUrl).toBe(`http://localhost:1234/${res.body.code}`);
  });

  it.each([
    ["missing url", {}],
    ["non-string url", { url: 123 }],
    ["not a url", { url: "not a url" }],
    ["unsupported scheme", { url: "javascript:alert(1)" }],
    ["ftp scheme", { url: "ftp://example.com/file" }],
    ["too long", { url: `https://example.com/${"a".repeat(3000)}` }],
  ])("rejects %s", async (_label, body) => {
    const res = await request(createApp(db)).post("/api/shorten").send(body);
    expect(res.status).toBe(400);
    expect(res.body.error).toBeDefined();
  });

  it("rejects malformed JSON", async () => {
    const res = await request(createApp(db))
      .post("/api/shorten")
      .set("Content-Type", "application/json")
      .send("{bad json");
    expect(res.status).toBe(400);
  });

  it("retries when a generated code collides", async () => {
    const codes = ["AAAAAA", "AAAAAA", "BBBBBB"];
    const app = createApp(db, { generateCode: () => codes.shift()! });

    const first = await request(app).post("/api/shorten").send({ url: "https://one.example" });
    const second = await request(app).post("/api/shorten").send({ url: "https://two.example" });

    expect(first.body.code).toBe("AAAAAA");
    expect(second.body.code).toBe("BBBBBB");
  });

  it("returns 503 if no free code can be found", async () => {
    const app = createApp(db, { generateCode: () => "AAAAAA" });
    await request(app).post("/api/shorten").send({ url: "https://one.example" });
    const res = await request(app).post("/api/shorten").send({ url: "https://two.example" });
    expect(res.status).toBe(503);
  });
});

describe("GET /:code", () => {
  it("redirects to the original url", async () => {
    const app = createApp(db);
    const { body } = await request(app).post("/api/shorten").send({ url: "https://example.com/target" });

    const res = await request(app).get(`/${body.code}`);
    expect(res.status).toBe(302);
    expect(res.headers.location).toBe("https://example.com/target");
  });

  it("returns 404 for an unknown code", async () => {
    const res = await request(createApp(db)).get("/ZZZZZZ");
    expect(res.status).toBe(404);
  });

  it("returns 404 for a malformed code", async () => {
    const res = await request(createApp(db)).get("/abc");
    expect(res.status).toBe(404);
  });

  it("treats codes as case-sensitive", async () => {
    const app = createApp(db, { generateCode: () => "AbCdEf" });
    await request(app).post("/api/shorten").send({ url: "https://example.com" });

    expect((await request(app).get("/AbCdEf")).status).toBe(302);
    expect((await request(app).get("/abcdef")).status).toBe(404);
  });

  it("persists links across app instances sharing a database", async () => {
    const { body } = await request(createApp(db)).post("/api/shorten").send({ url: "https://example.com/kept" });
    const res = await request(createApp(db)).get(`/${body.code}`);
    expect(res.headers.location).toBe("https://example.com/kept");
  });
});
