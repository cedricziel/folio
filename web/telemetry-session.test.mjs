import assert from "node:assert/strict";
import { test } from "node:test";
import {
  actionName,
  INACTIVITY_MS,
  MAX_SESSION_MS,
  ScreenTracker,
  screenName,
  SessionStore,
  UiEventLimiter,
} from "./telemetry-session.mjs";

class MemoryStorage {
  values = new Map();
  getItem(key) {
    return this.values.has(key) ? this.values.get(key) : null;
  }
  setItem(key, value) {
    this.values.set(key, String(value));
  }
  removeItem(key) {
    this.values.delete(key);
  }
}

class BlockedStorage {
  getItem() {
    throw new Error("SecurityError");
  }
  setItem() {
    throw new Error("SecurityError");
  }
  removeItem() {
    throw new Error("SecurityError");
  }
}

function fixture(storage = new MemoryStorage()) {
  let time = Date.UTC(2026, 10, 8, 12);
  let counter = 0;
  const store = new SessionStore({
    storage,
    now: () => time,
    randomId: () =>
      `00000000-0000-4000-8000-${String(++counter).padStart(12, "0")}`,
  });
  return {
    store,
    storage,
    advance: (ms) => {
      time += ms;
    },
  };
}

test("activity starts one session and keeps it while active", () => {
  const { store, advance } = fixture();
  const first = store.touch();
  assert.deepEqual(
    first.events.map((event) => event.name),
    ["session.start"],
  );
  assert.equal(first.events[0].previousId, undefined);
  for (let minute = 0; minute < 6; minute++) {
    advance(10 * 60 * 1000);
    const next = store.touch();
    assert.equal(next.id, first.id);
    assert.deepEqual(next.events, []);
  }
});

test("inactivity ends the session and links the next one", () => {
  const { store, advance } = fixture();
  const first = store.touch();
  advance(INACTIVITY_MS + 1);
  const next = store.touch();
  assert.notEqual(next.id, first.id);
  assert.deepEqual(next.events, [
    {
      name: "session.end",
      id: first.id,
      reason: "timeout",
      time: next.events[0].time,
    },
    { name: "session.start", id: next.id, previousId: first.id },
  ]);
  assert.equal(next.events[0].time, Date.UTC(2026, 10, 8, 12));
});

test("a session ends after four hours even with activity", () => {
  const { store, advance } = fixture();
  const first = store.touch();
  for (
    let elapsed = 0;
    elapsed < MAX_SESSION_MS - 1;
    elapsed += 20 * 60 * 1000
  ) {
    advance(Math.min(20 * 60 * 1000, MAX_SESSION_MS - 1 - elapsed));
    assert.equal(store.touch().id, first.id);
  }
  advance(1);
  const next = store.touch();
  assert.equal(next.events[0].reason, "max_duration");
  assert.equal(next.events[1].previousId, first.id);
});

test("a clock set back expires the session", () => {
  const { store, advance } = fixture();
  const first = store.touch();
  advance(-60 * 60 * 1000);
  const next = store.touch();
  assert.notEqual(next.id, first.id);
  assert.equal(next.events[0].reason, "timeout");
});

test("reading the current session never starts, extends or rotates it", () => {
  const { store, advance } = fixture();
  assert.equal(store.current(), undefined);
  const first = store.touch();
  advance(INACTIVITY_MS - 1);
  assert.equal(store.current(), first.id);
  advance(2);
  assert.equal(store.current(), undefined);
});

test("another tab sees the same session through storage", () => {
  const storage = new MemoryStorage();
  const one = fixture(storage);
  const two = new SessionStore({
    storage,
    now: () => Date.UTC(2026, 10, 8, 12),
  });
  const first = one.store.touch();
  const second = two.touch();
  assert.equal(second.id, first.id);
  assert.deepEqual(second.events, []);
});

test("ending a session reports the reason and forgets it", () => {
  const { store, storage } = fixture();
  const first = store.touch();
  assert.deepEqual(store.end("sign_out"), [
    {
      name: "session.end",
      id: first.id,
      reason: "sign_out",
      time: Date.UTC(2026, 10, 8, 12),
    },
  ]);
  assert.equal(storage.getItem("folio.telemetry.session"), null);
  const next = store.touch();
  assert.equal(next.events[0].previousId, undefined);
});

test("clearing on the sign-in page leaves no session behind", () => {
  const { store, storage } = fixture();
  store.touch();
  store.clear();
  assert.equal(storage.getItem("folio.telemetry.session"), null);
  assert.equal(store.current(), undefined);
});

test("blocked storage falls back to a session in memory", () => {
  const { store, advance } = fixture(new BlockedStorage());
  const first = store.touch();
  advance(1000);
  assert.equal(store.touch().id, first.id);
  assert.equal(store.current(), first.id);
});

test("a malformed stored record is replaced", () => {
  const storage = new MemoryStorage();
  storage.setItem("folio.telemetry.session", '{"id":"not-a-uuid"}');
  const { store } = fixture(storage);
  const session = store.touch();
  assert.match(session.id, /^[0-9a-f-]{36}$/);
  assert.equal(session.events[0].previousId, undefined);
});

test("rotation runs under the session lock when the browser has one", async () => {
  const { store } = fixture();
  const names = [];
  const locks = {
    request: async (name, callback) => {
      names.push(name);
      return callback();
    },
  };
  const session = await store.touchLocked(locks);
  assert.equal(session.events[0].name, "session.start");
  assert.deepEqual(names, ["folio.telemetry.session"]);
  assert.equal((await store.touchLocked(undefined)).id, session.id);
});

test("screen names outside the vocabulary become other", () => {
  assert.equal(screenName("inbox"), "inbox");
  assert.equal(screenName("document.detail"), "document.detail");
  assert.equal(screenName("/documents/1234"), "other");
  assert.equal(screenName(undefined), "other");
});

test("action names outside the vocabulary are refused", () => {
  assert.equal(actionName("document.delete"), "document.delete");
  assert.equal(actionName("document.secret"), undefined);
  assert.equal(actionName(""), undefined);
});

test("a repeated screen within a second is not reported again", () => {
  let time = 0;
  const screens = new ScreenTracker(() => time);
  assert.equal(screens.view("inbox"), "inbox");
  time += 500;
  assert.equal(screens.view("inbox"), undefined);
  time += 1000;
  assert.equal(screens.view("inbox"), "inbox");
  assert.equal(screens.view("trash"), "trash");
  assert.equal(screens.current, "trash");
});

test("at most sixty UI events per minute are allowed", () => {
  let time = 0;
  const limiter = new UiEventLimiter(() => time);
  let allowed = 0;
  for (let i = 0; i < 100; i++) if (limiter.allow()) allowed++;
  assert.equal(allowed, 60);
  assert.equal(limiter.takeDropped(), 40);
  assert.equal(limiter.takeDropped(), 0);
  time += 60_001;
  assert.equal(limiter.allow(), true);
});
