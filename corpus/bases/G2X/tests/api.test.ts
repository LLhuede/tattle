import request from "supertest";
import { beforeEach, describe, expect, it } from "vitest";
import { createApp } from "../src/app";
import { openDb } from "../src/db";

let app: ReturnType<typeof createApp>;

beforeEach(() => {
  app = createApp(openDb(":memory:"));
});

async function addPerson(name: string): Promise<number> {
  const res = await request(app).post("/people").send({ name }).expect(201);
  return res.body.id;
}

describe("API", () => {
  it("settles up a dinner via the HTTP endpoints", async () => {
    const alice = await addPerson("Alice");
    const bob = await addPerson("Bob");
    const carol = await addPerson("Carol");

    await request(app)
      .post("/expenses")
      .send({ description: "Dinner", amount: 100, paidBy: alice, splitBetween: [alice, bob, carol] })
      .expect(201);

    const res = await request(app).get("/settle-up").expect(200);
    expect(res.body.balances).toEqual([
      { personId: alice, name: "Alice", balance: 66.66 },
      { personId: bob, name: "Bob", balance: -33.33 },
      { personId: carol, name: "Carol", balance: -33.33 },
    ]);
    expect(res.body.transfers).toEqual([
      { from: { id: bob, name: "Bob" }, to: { id: alice, name: "Alice" }, amount: 33.33 },
      { from: { id: carol, name: "Carol" }, to: { id: alice, name: "Alice" }, amount: 33.33 },
    ]);
  });

  it("removes a deleted expense from the settle up", async () => {
    const alice = await addPerson("Alice");
    const bob = await addPerson("Bob");
    const { body } = await request(app)
      .post("/expenses")
      .send({ description: "Taxi", amount: 20, paidBy: alice, splitBetween: [alice, bob] })
      .expect(201);

    await request(app).delete(`/expenses/${body.id}`).expect(204);
    const res = await request(app).get("/settle-up").expect(200);
    expect(res.body.transfers).toEqual([]);
  });

  it("validates input", async () => {
    const alice = await addPerson("Alice");
    await request(app).post("/people").send({ name: "Alice" }).expect(409);
    await request(app).post("/people").send({}).expect(400);

    const bad = [
      { description: "x", amount: -5, paidBy: alice, splitBetween: [alice] },
      { description: "x", amount: 1.234, paidBy: alice, splitBetween: [alice] },
      { description: "x", amount: 5, paidBy: 999, splitBetween: [alice] },
      { description: "x", amount: 5, paidBy: alice, splitBetween: [] },
      { description: "x", amount: 5, paidBy: alice, splitBetween: [999] },
      { amount: 5, paidBy: alice, splitBetween: [alice] },
    ];
    for (const body of bad) {
      await request(app).post("/expenses").send(body).expect(400);
    }
  });
});
