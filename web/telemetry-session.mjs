// Session and UI-event rules for browser telemetry. Pure: no side effects on import.
// Names follow OpenTelemetry semantic conventions v1.45.0 (session.*, app.screen.name; Development status).

export const SESSION_KEY = "folio.telemetry.session";
export const INACTIVITY_MS = 30 * 60 * 1000;
export const MAX_SESSION_MS = 4 * 60 * 60 * 1000;

const uuid =
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

export const SCREENS = new Set([
  "onboarding",
  "app_lock",
  "inbox",
  "documents",
  "saved_view",
  "saved_views",
  "saved_view.editor",
  "document.detail",
  "document.preview",
  "document.discussion",
  "document.processing",
  "scan.review",
  "upload.review",
  "taxonomy",
  "sources",
  "source.outcomes",
  "trash",
  "workflows",
  "integrations",
  "settings",
  "settings.security",
  "settings.vaults",
  "share",
  "error",
  "other",
]);

export const ACTIONS = new Set([
  "document.open",
  "document.upload",
  "document.delete",
  "document.restore",
  "document.purge",
  "document.review",
  "document.download",
  "document.share",
  "document.reprocess",
  "document.edit",
  "document.tag.add",
  "document.tag.remove",
  "discussion.post",
  "browse.scope.change",
  "browse.filter.change",
  "browse.load_more",
  "browse.refresh",
  "search.submit",
  "saved_view.open",
  "saved_view.save",
  "saved_view.delete",
  "vault.switch",
  "scan.start",
  "scan.finish",
  "scan.cancel",
  "share.submit",
  "share.cancel",
  "auth.sign_out",
  "app_lock.unlock",
  "intent.run",
]);

export const UPLOAD_SOURCES = new Set([
  "picker",
  "scan",
  "drop",
  "paste",
  "share",
  "intent",
]);

const LOG_ATTRIBUTES = new Set([
  "http.route",
  "folio.navigation.type",
  "session.id",
  "session.previous_id",
  "folio.session.end_reason",
  "app.screen.name",
  "folio.action.name",
  "folio.upload.source",
]);

const LOG_EVENTS = new Set([
  "session.start",
  "session.end",
  "folio.screen.view",
  "folio.action",
]);

/** Strips a log record to the fixed vocabulary before export. */
export function sanitizeLog(record) {
  for (const key of Object.keys(record.attributes))
    if (!LOG_ATTRIBUTES.has(key)) delete record.attributes[key];
  if (record.eventName !== undefined && !LOG_EVENTS.has(record.eventName))
    record.setEventName(undefined);
  if (record.body !== "folio.web.navigation") record.setBody(undefined);
}

export function uploadSource(value) {
  return UPLOAD_SOURCES.has(value) ? value : undefined;
}

export function screenName(value) {
  return SCREENS.has(value) ? value : "other";
}

export function actionName(value) {
  return ACTIONS.has(value) ? value : undefined;
}

export function randomId() {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID();
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function valid(record) {
  return (
    record &&
    uuid.test(record.id) &&
    Number.isFinite(record.startedAt) &&
    Number.isFinite(record.lastActivityAt)
  );
}

/** Why `record` is no longer live at `now`, or undefined while it is. */
function expiry(record, now) {
  if (now < record.lastActivityAt || now < record.startedAt) return "timeout";
  if (now - record.startedAt >= MAX_SESSION_MS) return "max_duration";
  if (now - record.lastActivityAt > INACTIVITY_MS) return "timeout";
  return undefined;
}

/** One session per browser profile, shared by tabs through storage. */
export class SessionStore {
  #storage;
  #now;
  #randomId;
  #memory;
  #storageFailed = false;

  constructor({ storage, now = () => Date.now(), randomId: id = randomId }) {
    this.#storage = storage;
    this.#now = now;
    this.#randomId = id;
  }

  #read() {
    if (this.#storageFailed) return this.#memory;
    try {
      const saved = this.#storage?.getItem(SESSION_KEY);
      if (saved === null || saved === undefined) {
        this.#memory = undefined;
        return undefined;
      }
      const record = JSON.parse(saved);
      this.#memory = valid(record) ? record : undefined;
    } catch {
      // Blocked or corrupt storage: keep the in-memory session for this page.
    }
    return this.#memory;
  }

  #write(record) {
    this.#memory = record;
    try {
      if (record) this.#storage?.setItem(SESSION_KEY, JSON.stringify(record));
      else this.#storage?.removeItem(SESSION_KEY);
    } catch {
      // Storage that refuses writes would forget every session; keep this page's in memory from now on.
      this.#storageFailed = true;
    }
  }

  /** The live session ID, without counting as activity. Reads the cached copy; see `refresh`. */
  current() {
    const record = this.#memory;
    if (!record || expiry(record, this.#now())) return undefined;
    return record.id;
  }

  /** Reloads the cached copy, for example when another tab changed the session. */
  refresh() {
    this.#read();
  }

  /** Records user activity: extends the live session or rotates an expired one. */
  touch() {
    const now = this.#now();
    const record = this.#read();
    const events = [];
    if (record && !expiry(record, now)) {
      this.#write({ ...record, lastActivityAt: now });
      return { id: record.id, events };
    }
    const next = {
      id: this.#randomId(),
      startedAt: now,
      lastActivityAt: now,
    };
    if (record) {
      next.previousId = record.id;
      events.push({
        name: "session.end",
        id: record.id,
        reason: expiry(record, now),
        time: record.lastActivityAt,
      });
    }
    events.push({
      name: "session.start",
      id: next.id,
      previousId: next.previousId,
    });
    this.#write(next);
    return { id: next.id, events };
  }

  /** `touch` under a lock shared by the profile's tabs, where the browser has Web Locks. */
  async touchLocked(locks) {
    if (typeof locks?.request !== "function") return this.touch();
    return locks.request(SESSION_KEY, () => this.touch());
  }

  /** Ends the session for `reason` and forgets it, so the next one is not linked. */
  end(reason) {
    const record = this.#read();
    this.#write(undefined);
    if (!record || expiry(record, this.#now())) return [];
    return [{ name: "session.end", id: record.id, reason, time: this.#now() }];
  }

  clear() {
    this.#write(undefined);
  }
}

/** Reports screen changes, ignoring the same screen again within a second. */
export class ScreenTracker {
  #now;
  #last;
  #at = -Infinity;
  current;

  constructor(now = () => performance.now()) {
    this.#now = now;
  }

  view(name) {
    const now = this.#now();
    if (name === this.#last && now - this.#at < 1000) return undefined;
    this.#last = name;
    this.#at = now;
    this.current = name;
    return name;
  }
}

/** Allows at most `limit` UI events in any rolling minute and counts the rest. */
export class UiEventLimiter {
  #now;
  #limit;
  #times = [];
  #dropped = 0;

  constructor(now = () => performance.now(), limit = 60) {
    this.#now = now;
    this.#limit = limit;
  }

  allow() {
    const now = this.#now();
    while (this.#times.length && now - this.#times[0] > 60_000)
      this.#times.shift();
    if (this.#times.length >= this.#limit) {
      this.#dropped++;
      return false;
    }
    this.#times.push(now);
    return true;
  }

  takeDropped() {
    const dropped = this.#dropped;
    this.#dropped = 0;
    return dropped;
  }
}
