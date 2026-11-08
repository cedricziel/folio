import { context, propagation, ROOT_CONTEXT, trace } from "@opentelemetry/api";
import { W3CTraceContextPropagator } from "@opentelemetry/core";
import { resourceFromAttributes } from "@opentelemetry/resources";
import {
  BatchSpanProcessor,
  StackContextManager,
  TracerProvider,
} from "@opentelemetry/sdk-trace";
import {
  MeterProvider,
  PeriodicExportingMetricReader,
} from "@opentelemetry/sdk-metrics";
import {
  BatchLogRecordProcessor,
  LoggerProvider,
} from "@opentelemetry/sdk-logs";
import { OTLPTraceExporter } from "@opentelemetry/exporter-trace-otlp-http";
import { OTLPMetricExporter } from "@opentelemetry/exporter-metrics-otlp-http";
import { OTLPLogExporter } from "@opentelemetry/exporter-logs-otlp-http";
import { registerInstrumentations } from "@opentelemetry/instrumentation";
import { FetchInstrumentation } from "@opentelemetry/instrumentation-fetch";
import { XMLHttpRequestInstrumentation } from "@opentelemetry/instrumentation-xml-http-request";
import {
  actionName,
  randomId,
  ScreenTracker,
  screenName,
  SessionStore,
  UiEventLimiter,
} from "./telemetry-session.mjs";

const configFetch = window.fetch.bind(window);
const installationKey = "folio.telemetry.installation.id";
const routeSegments = new Set([
  "api",
  "documents",
  "upload",
  "search",
  "download",
  "thumbnail",
  "delete",
  "correspondent",
  "correspondents",
  "document_type",
  "document_types",
  "tags",
  "reprocess",
  "settings",
  "security",
  "password",
  "totp",
  "start",
  "confirm",
  "cancel",
  "disable",
  "recovery",
  "passkeys",
  "register",
  "login",
  "logout",
  "setup",
  "auth",
  "oauth",
  "authorize",
  "consent",
  "token",
  "manage",
  "create",
  "edit",
  "update",
  "sources",
  "new",
  "toggle",
  "test",
  "ai",
  "processing",
  "reembed",
  "mcp",
  "workflows",
  "rules",
  "actions",
  "runs",
  "events",
  "healthz",
  "readyz",
  "openapi.json",
  "docs",
  "userinfo",
  "connect",
  "callback",
]);
let enabled = false;
let starting;
let telemetry;
let navigationStart;
let initialLoad = true;
const sessionPattern =
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const sessions = new SessionStore({
  storage: {
    getItem: (key) => localStorage.getItem(key),
    setItem: (key, value) => localStorage.setItem(key, value),
    removeItem: (key) => localStorage.removeItem(key),
  },
});
const screens = new ScreenTracker();
const limiter = new UiEventLimiter();
const logAttributes = new Set([
  "http.route",
  "folio.navigation.type",
  "session.id",
  "session.previous_id",
  "folio.session.end_reason",
  "app.screen.name",
  "folio.action.name",
]);
const logEvents = new Set([
  "session.start",
  "session.end",
  "folio.screen.view",
  "folio.action",
]);

function routeFor(value) {
  try {
    const url = new URL(value, location.origin);
    if (url.origin !== location.origin) return "/external";
    return url.pathname
      .split("/")
      .map((part) => {
        if (!part || routeSegments.has(part)) return part;
        return ":id";
      })
      .join("/");
  } catch {
    return "/unknown";
  }
}

function installationId() {
  const generated = randomId();
  try {
    const saved = localStorage.getItem(installationKey);
    if (
      saved &&
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(
        saved,
      )
    )
      return saved;
    localStorage.setItem(installationKey, generated);
  } catch {
    // Storage can be blocked in private browsing; keep this page usable.
  }
  return generated;
}

function deviceAttributes() {
  const agent = navigator.userAgent;
  const browser = agent.match(/(Edg|Firefox|Chrome|Version)\/(\d+)/);
  const names = {
    Edg: "Edge",
    Firefox: "Firefox",
    Chrome: "Chrome",
    Version: "Safari",
  };
  const attributes = {
    "device.type": /iPad|Tablet/.test(agent)
      ? "tablet"
      : /Mobile/.test(agent)
        ? "mobile"
        : "desktop",
  };
  if (browser) {
    attributes["browser.name"] = names[browser[1]];
    attributes["browser.version"] = browser[2];
  }
  const os = /Android/.test(agent)
    ? "android"
    : /iPhone|iPad|iPod/.test(agent)
      ? "ios"
      : /Windows/.test(agent)
        ? "windows"
        : /Macintosh/.test(agent)
          ? "darwin"
          : /Linux/.test(agent)
            ? "linux"
            : undefined;
  if (os) attributes["os.type"] = os;
  return attributes;
}

function safeSpan(span) {
  const source = span.attributes;
  const attributes = {};
  const method = source["http.request.method"] || source["http.method"];
  if (typeof method === "string")
    attributes["http.request.method"] =
      /^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)$/.test(method)
        ? method
        : "_OTHER";
  const status =
    source["http.response.status_code"] ?? source["http.status_code"];
  if (typeof status === "number")
    attributes["http.response.status_code"] = status;
  const url = source["url.full"] || source["http.url"];
  if (typeof url === "string") attributes["http.route"] = routeFor(url);
  if (typeof source["http.route"] === "string")
    attributes["http.route"] = routeFor(source["http.route"]);
  if (typeof source["folio.navigation.type"] === "string")
    attributes["folio.navigation.type"] = source["folio.navigation.type"];
  if (sessionPattern.test(source["session.id"]))
    attributes["session.id"] = source["session.id"];
  if (typeof source["app.screen.name"] === "string")
    attributes["app.screen.name"] = screenName(source["app.screen.name"]);
  return {
    ...span,
    spanContext: () => span.spanContext(),
    name: method
      ? `HTTP ${attributes["http.request.method"]}`
      : "folio.navigation",
    attributes,
    events: [],
    links: [],
    status: { code: span.status.code },
  };
}

/** Adds the session and current screen to every span and log record. */
function stamp(record) {
  const id = sessions.current();
  if (id && record.attributes["session.id"] === undefined)
    record.setAttribute("session.id", id);
  if (screens.current && record.attributes["app.screen.name"] === undefined)
    record.setAttribute("app.screen.name", screens.current);
}

const spanStamper = {
  onStart: stamp,
  onEnd() {},
  forceFlush: () => Promise.resolve(),
  shutdown: () => Promise.resolve(),
};

/** Stamps, then keeps only the fixed log vocabulary before export. */
const logSanitizer = {
  onEmit(record) {
    stamp(record);
    for (const key of Object.keys(record.attributes))
      if (!logAttributes.has(key)) delete record.attributes[key];
    if (record.eventName !== undefined && !logEvents.has(record.eventName))
      record.setEventName(undefined);
    if (record.body !== "folio.web.navigation") record.setBody(undefined);
  },
  forceFlush: () => Promise.resolve(),
  shutdown: () => Promise.resolve(),
};

/** The server request span named in Server-Timing on a full page load. */
function serverContext() {
  try {
    const [entry] = performance.getEntriesByType("navigation");
    const value = entry?.serverTiming?.find(
      (timing) => timing.name === "traceparent",
    )?.description;
    const match = /^00-([0-9a-f]{32})-([0-9a-f]{16})-0([01])$/.exec(value);
    if (!match) return ROOT_CONTEXT;
    return trace.setSpanContext(ROOT_CONTEXT, {
      traceId: match[1],
      spanId: match[2],
      traceFlags: Number(match[3]),
      isRemote: true,
    });
  } catch {
    return ROOT_CONTEXT;
  }
}

function gatedExporter(exporter, sanitize = (value) => value) {
  return {
    export(value, callback) {
      if (!enabled) {
        callback({ code: 0 });
        return;
      }
      exporter.export(sanitize(value), callback);
    },
    forceFlush: () => exporter.forceFlush(),
    shutdown: () => exporter.shutdown(),
    selectAggregationTemporality:
      exporter.selectAggregationTemporality?.bind(exporter),
    selectAggregation: exporter.selectAggregation?.bind(exporter),
  };
}

function createTelemetry(config) {
  const resource = resourceFromAttributes({
    "service.name": "folio-web",
    "service.namespace": "folio",
    "service.version": config.serviceVersion,
    "service.instance.id": randomId(),
    "folio.installation.id": installationId(),
    ...deviceAttributes(),
  });
  const exporterOptions = (signal) => ({
    url: `${location.origin}/api/telemetry/v1/${signal}`,
    timeoutMillis: 3000,
    concurrencyLimit: 1,
  });
  const batchOptions = {
    maxQueueSize: 128,
    maxExportBatchSize: 16,
    scheduledDelayMillis: 5000,
    exportTimeoutMillis: 3500,
  };
  const traces = new TracerProvider({
    resource,
    spanLimits: {
      attributeCountLimit: 16,
      attributeValueLengthLimit: 256,
      eventCountLimit: 0,
      linkCountLimit: 0,
    },
    spanProcessors: [
      spanStamper,
      new BatchSpanProcessor({
        ...batchOptions,
        exporter: gatedExporter(
          new OTLPTraceExporter(exporterOptions("traces")),
          (spans) => spans.map(safeSpan),
        ),
      }),
    ],
  });
  context.setGlobalContextManager(new StackContextManager().enable());
  propagation.setGlobalPropagator(new W3CTraceContextPropagator());
  trace.setGlobalTracerProvider(traces);
  const meters = new MeterProvider({
    resource,
    readers: [
      new PeriodicExportingMetricReader({
        exporter: gatedExporter(
          new OTLPMetricExporter(exporterOptions("metrics")),
        ),
        exportIntervalMillis: 10000,
        exportTimeoutMillis: 3500,
      }),
    ],
  });
  const logs = new LoggerProvider({
    resource,
    processors: [
      logSanitizer,
      new BatchLogRecordProcessor({
        ...batchOptions,
        exporter: gatedExporter(new OTLPLogExporter(exporterOptions("logs"))),
      }),
    ],
  });
  const origin = location.origin.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const instrumentationConfig = {
    ignoreUrls: [
      new RegExp(`^(?!${origin}/)`),
      new RegExp(`^${origin}/api/telemetry(?:[/?#]|$)`),
    ],
    propagateTraceHeaderCorsUrls: [],
    ignoreNetworkEvents: true,
    measureRequestSize: false,
  };
  const instrumentations = [
    new FetchInstrumentation(instrumentationConfig),
    new XMLHttpRequestInstrumentation(instrumentationConfig),
  ];
  registerInstrumentations({ tracerProvider: traces, instrumentations });
  const meter = meters.getMeter("folio.web");
  const navigations = meter.createCounter("folio.web.navigation.count");
  const dropped = meter.createCounter("folio.telemetry.ui_events.dropped");
  const duration = meter.createHistogram("folio.web.navigation.duration", {
    unit: "s",
  });
  const logger = logs.getLogger("folio.web");
  const tracer = traces.getTracer("folio.web");

  function event(eventName, attributes = {}, timestamp) {
    logger.emit({
      eventName,
      severityNumber: 9,
      severityText: "INFO",
      attributes,
      timestamp,
    });
  }

  function sessionEvents(events) {
    for (const item of events) {
      const attributes = { "session.id": item.id };
      if (item.previousId) attributes["session.previous_id"] = item.previousId;
      if (item.reason) attributes["folio.session.end_reason"] = item.reason;
      event(item.name, attributes, item.time);
    }
  }

  function uiEvent(eventName, attributes) {
    if (limiter.allow()) event(eventName, attributes);
    const count = limiter.takeDropped();
    if (count) dropped.add(count);
  }

  function flush() {
    return Promise.allSettled([
      traces.forceFlush(),
      meters.forceFlush(),
      logs.forceFlush(),
    ]);
  }

  async function screen() {
    const name = screenName(
      document.querySelector('meta[name="folio-screen"]')?.content,
    );
    let session;
    try {
      session = await sessions.touchLocked(navigator.locks);
    } catch {
      session = sessions.touch();
    }
    sessionEvents(session.events);
    if (screens.view(name)) uiEvent("folio.screen.view", { "app.screen.name": name });
  }

  function action(name) {
    sessionEvents(sessions.touch().events);
    uiEvent("folio.action", { "folio.action.name": name });
    if (name === "auth.sign_out") sessionEvents(sessions.end("sign_out"));
    void flush();
  }

  function navigation(type, start = performance.now()) {
    if (!enabled) return;
    const route = routeFor(location.href);
    const elapsed = Math.max(0, performance.now() - start);
    const span = tracer.startSpan(
      "folio.navigation",
      {
        startTime: performance.timeOrigin + start,
        attributes: { "http.route": route, "folio.navigation.type": type },
      },
      type === "document" ? serverContext() : ROOT_CONTEXT,
    );
    const spanContext = trace.setSpan(ROOT_CONTEXT, span);
    const attributes = { "http.route": route, "folio.navigation.type": type };
    navigations.add(1, attributes, spanContext);
    duration.record(elapsed / 1000, attributes, spanContext);
    logger.emit({
      body: "folio.web.navigation",
      severityNumber: 9,
      severityText: "INFO",
      attributes,
      context: spanContext,
    });
    span.end();
  }

  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden" && enabled) void flush();
  });
  addEventListener("pagehide", () => {
    if (enabled) void flush();
  });
  return { instrumentations, navigation, screen, action };
}

function isSignInPage() {
  return /^\/(?:login|setup)(?:\/|$)/.test(location.pathname);
}

function declaredAction(element) {
  return actionName(element?.getAttribute?.("data-folio-action") ?? undefined);
}

// Bubble phase: a cancelled confirmation stops propagation and records nothing.
document.addEventListener("submit", (submitted) => {
  const name = declaredAction(submitted.target);
  if (enabled && name) telemetry.action(name);
});
document.addEventListener("click", (clicked) => {
  const element = clicked.target?.closest?.("[data-folio-action]");
  if (!element || element.tagName === "FORM") return;
  const name = declaredAction(element);
  if (enabled && name) telemetry.action(name);
});
// Scripted actions (uploads, tags, vault switch) announce themselves by name.
document.addEventListener("folio:action", (announced) => {
  const name = actionName(announced.detail);
  if (enabled && name) telemetry.action(name);
});

async function configure() {
  if (isSignInPage()) {
    sessions.clear();
    enabled = false;
    for (const instrumentation of telemetry?.instrumentations ?? [])
      instrumentation.disable();
    return;
  }
  if (starting) return starting;
  starting = (async () => {
    try {
      const response = await configFetch("/api/telemetry/config", {
        credentials: "same-origin",
        cache: "no-store",
        signal: AbortSignal.timeout(3000),
      });
      const config = response.ok ? await response.json() : {};
      enabled =
        !isSignInPage() &&
        config.enabled === true &&
        config.endpoint === "/api/telemetry" &&
        config.serviceNamespace === "folio" &&
        typeof config.serviceVersion === "string";
      if (enabled && !telemetry) telemetry = createTelemetry(config);
      for (const instrumentation of telemetry?.instrumentations ?? []) {
        if (enabled) instrumentation.enable();
        else instrumentation.disable();
      }
      if (enabled) {
        await telemetry.screen();
        telemetry.navigation(
          initialLoad ? "document" : "turbo",
          initialLoad ? 0 : navigationStart,
        );
        initialLoad = false;
      }
    } catch {
      enabled = false;
      for (const instrumentation of telemetry?.instrumentations ?? [])
        instrumentation.disable();
    } finally {
      starting = undefined;
      navigationStart = undefined;
    }
  })();
  return starting;
}

document.addEventListener("turbo:before-visit", () => {
  navigationStart = performance.now();
});
document.addEventListener("turbo:load", () => {
  void configure();
});
void configure();
