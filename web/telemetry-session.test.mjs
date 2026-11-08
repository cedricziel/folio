import assert from "node:assert/strict";
import { test } from "node:test";
import {
  actionName,
  sanitizeLog,
  uploadSource,
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

class ReadOnlyStorage extends MemoryStorage {
  setItem() {
    throw new Error("QuotaExceededError");
  }
}

test("storage that refuses writes keeps one session in memory", () => {
  const { store, advance } = fixture(new ReadOnlyStorage());
  const first = store.touch();
  advance(1000);
  const second = store.touch();
  assert.equal(second.id, first.id);
  assert.deepEqual(second.events, []);
  assert.equal(store.current(), first.id);
});

test("current reads the cached session until storage is refreshed", () => {
  const storage = new MemoryStorage();
  const one = fixture(storage);
  const two = new SessionStore({ storage, now: () => Date.UTC(2026, 10, 8, 12) });
  const first = one.store.touch();
  assert.equal(two.current(), undefined);
  two.refresh();
  assert.equal(two.current(), first.id);
  storage.removeItem("folio.telemetry.session");
  assert.equal(two.current(), first.id);
  two.refresh();
  assert.equal(two.current(), undefined);
});

test("signing out ends the live session without rotating an expired one", () => {
  const { store, advance } = fixture();
  const first = store.touch();
  assert.deepEqual(
    store.end("sign_out").map((event) => [event.name, event.reason]),
    [["session.end", "sign_out"]],
  );
  const next = store.touch();
  assert.notEqual(next.id, first.id);
  assert.equal(next.events[0].previousId, undefined);
  advance(INACTIVITY_MS + 1);
  assert.deepEqual(store.end("sign_out"), []);
  assert.equal(store.current(), undefined);
});

test("the session lock lets one tab rotate at a time", async () => {
  const storage = new MemoryStorage();
  let time = Date.UTC(2026, 10, 8, 12);
  const now = () => time;
  const tabs = [new SessionStore({ storage, now }), new SessionStore({ storage, now })];
  tabs[0].touch();
  time += INACTIVITY_MS + 1;
  let holders = 0;
  let most = 0;
  let queue = Promise.resolve();
  const locks = {
    request(_name, callback) {
      const run = queue.then(async () => {
        holders++;
        most = Math.max(most, holders);
        await new Promise((resolve) => setTimeout(resolve, 1));
        try {
          return callback();
        } finally {
          holders--;
        }
      });
      queue = run.catch(() => {});
      return run;
    },
  };
  const results = await Promise.all(
    Array.from({ length: 10 }, (_, i) => tabs[i % 2].touchLocked(locks)),
  );
  assert.equal(most, 1);
  assert.equal(new Set(results.map((result) => result.id)).size, 1);
  assert.equal(
    results.flatMap((result) => result.events).filter((e) => e.name === "session.start").length,
    1,
  );
});

test("log records keep only the fixed vocabulary", () => {
  const record = {
    attributes: {
      "session.id": "00000000-0000-4000-8000-000000000001",
      "folio.action.name": "document.delete",
      "folio.upload.source": "drop",
      "url.full": "https://folio.example/documents/secret",
      "document.title": "private-title",
    },
    eventName: "folio.action",
    body: "private-body",
    setEventName(value) {
      this.eventName = value;
    },
    setBody(value) {
      this.body = value;
    },
  };
  sanitizeLog(record);
  assert.deepEqual(Object.keys(record.attributes).sort(), [
    "folio.action.name",
    "folio.upload.source",
    "session.id",
  ]);
  assert.equal(record.body, undefined);
  assert.equal(record.eventName, "folio.action");

  const navigation = { ...record, attributes: {}, eventName: "custom.event", body: "folio.web.navigation" };
  sanitizeLog(navigation);
  assert.equal(navigation.eventName, undefined);
  assert.equal(navigation.body, "folio.web.navigation");
});

test("upload sources outside the vocabulary are dropped", () => {
  assert.equal(uploadSource("drop"), "drop");
  assert.equal(uploadSource("/Users/me/secret.pdf"), undefined);
  assert.equal(actionName("browse.sort.change"), undefined);
});
