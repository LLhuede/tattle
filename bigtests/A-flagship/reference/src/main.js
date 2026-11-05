import { formatCents, parseAmount } from "./lib/money.js";

const app = document.getElementById("app");
let token = localStorage.getItem("token");

async function api(method, path, body) {
  const r = await fetch("/api" + path, {
    method,
    headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = r.status === 204 ? null : await r.json();
  if (!r.ok) throw new Error(data.error);
  return data;
}

function el(tag, props = {}, ...kids) {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...kids);
  return e;
}

function authView() {
  const email = el("input", { placeholder: "email" });
  const name = el("input", { placeholder: "name" });
  const password = el("input", { placeholder: "password", type: "password" });
  const msg = el("p");
  const go = (path) => async () => {
    try {
      const r = await api("POST", path, { email: email.value, name: name.value, password: password.value });
      token = r.token;
      localStorage.setItem("token", token);
      render();
    } catch (e) {
      msg.textContent = e.message;
    }
  };
  app.replaceChildren(el("h1", {}, "Shared Expenses"), email, name, password,
    el("button", { onclick: go("/signup") }, "Sign up"), el("button", { onclick: go("/login") }, "Log in"), msg);
}

async function groupView(id) {
  const [g, bal, settle, act, ex] = await Promise.all([
    api("GET", `/groups/${id}`), api("GET", `/groups/${id}/balances`), api("GET", `/groups/${id}/settle-up`),
    api("GET", `/groups/${id}/activity`), api("GET", `/groups/${id}/expenses`),
  ]);
  const who = (uid) => g.members.find((m) => m.id === uid)?.name ?? uid;
  const desc = el("input", { placeholder: "description" });
  const amt = el("input", { placeholder: "12.50" });
  const add = el("button", {
    onclick: async () => {
      const cents = parseAmount(amt.value);
      if (cents === null || cents === 0) return alert("bad amount");
      const me = (await api("GET", "/me")).user.id;
      await api("POST", `/groups/${id}/expenses`, { description: desc.value, amountCents: cents, paidBy: me, split: { type: "equal", participants: g.members.map((m) => m.id) } });
      groupView(id);
    },
  }, "Add expense (equal split)");
  app.replaceChildren(
    el("button", { onclick: render }, "Back"), el("h2", {}, g.name),
    el("h3", {}, "Balances"), ...bal.balances.map((b) => el("div", {}, `${who(b.userId)}: ${formatCents(b.balanceCents)}`)),
    el("h3", {}, "Settle up"), ...settle.transfers.map((t) => el("div", {}, `${who(t.from)} pays ${who(t.to)} ${formatCents(t.amountCents)}`)),
    el("h3", {}, "Expenses"), ...ex.expenses.map((e) => el("div", {}, `${e.description}: ${formatCents(e.amountCents)}`)), desc, amt, add,
    el("h3", {}, "Activity"), ...act.activity.map((a) => el("div", {}, `${a.type} by ${who(a.actorId)}`)),
  );
}

async function render() {
  if (!token) return authView();
  try {
    const { groups } = await api("GET", "/groups");
    const name = el("input", { placeholder: "new group" });
    app.replaceChildren(el("h1", {}, "Your groups"),
      ...groups.map((g) => el("div", {}, el("a", { href: "#", onclick: (e) => { e.preventDefault(); groupView(g.id); } }, g.name))),
      name, el("button", { onclick: async () => { await api("POST", "/groups", { name: name.value }); render(); } }, "Create"),
      el("button", { onclick: () => { token = null; localStorage.removeItem("token"); render(); } }, "Sign out"));
  } catch {
    token = null;
    authView();
  }
}

render();
