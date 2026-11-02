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

function randomId() {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID();
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
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
  const duration = meter.createHistogram("folio.web.navigation.duration", {
    unit: "s",
  });
  const logger = logs.getLogger("folio.web");
  const tracer = traces.getTracer("folio.web");

  function navigation(type, start = performance.now()) {
    if (!enabled) return;
    const route = routeFor(location.href);
    const elapsed = Math.max(0, performance.now() - start);
    const span = tracer.startSpan("folio.navigation", {
      startTime: performance.timeOrigin + start,
      attributes: { "http.route": route, "folio.navigation.type": type },
    });
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
    if (document.visibilityState === "hidden" && enabled) {
      void Promise.allSettled([
        traces.forceFlush(),
        meters.forceFlush(),
        logs.forceFlush(),
      ]);
    }
  });
  return { instrumentations, navigation };
}

async function configure() {
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
