import { execFileSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import http2 from "node:http2";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  expect,
  request as playwrightRequest,
  test,
  type APIRequestContext,
} from "@playwright/test";
import { samplePdf } from "./fixtures";

// A throwaway P-256 key; the stub below accepts any signature.
const KEY = `-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgAosoCcYoVHPZlCWP
oShkVBX+C6hjdfOCLb99IVGjenWhRANCAAQHz9RTjiH1cvtgtneWb5gLPuVYpIpN
KAuAQloidgaveNCZtHQFTnDTR4pFwYBKElRLGQnBgDU8trF2AC/LsCzk
-----END PRIVATE KEY-----
`;
const scripts = path.resolve(__dirname, "../../.claude/skills/verify/scripts");
const DEVICE_TOKEN = "ab".repeat(32);
const REDIRECT = "http://127.0.0.1:9/callback";

type Pushed = {
  path: string;
  headers: http2.IncomingHttpHeaders;
  body: string;
};

/** An APNs stand-in speaking cleartext HTTP/2 on a free port */
async function apnsStub(): Promise<{
  url: string;
  pushes: Pushed[];
  close: () => void;
}> {
  const pushes: Pushed[] = [];
  const server = http2.createServer();
  server.on("stream", (stream, headers) => {
    let body = "";
    stream.on("data", (chunk) => (body += chunk));
    stream.on("end", () => {
      pushes.push({ path: String(headers[":path"]), headers, body });
      stream.respond({ ":status": 200 });
      stream.end();
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as { port: number };
  return {
    url: `http://127.0.0.1:${port}`,
    pushes,
    close: () => server.close(),
  };
}

function csrfFrom(html: string): string {
  const match = html.match(/name="csrf" value="([^"]*)"/);
  if (!match) throw new Error("no csrf field");
  return match[1];
}

/** Create the owner, then sign an app in through OAuth with PKCE; returns its access token */
async function signInApp(
  api: APIRequestContext,
  base: string,
  setupToken: string,
): Promise<string> {
  const setup = await api.get(`/setup?token=${setupToken}`);
  const created = await api.post("/setup", {
    form: {
      csrf: csrfFrom(await setup.text()),
      token: setupToken,
      username: "owner",
      password: "push-e2e-password-123",
    },
    maxRedirects: 0,
  });
  expect(created.status()).toBe(303);

  const registered = await api.post("/oauth/register", {
    data: {
      client_name: "Folio",
      redirect_uris: [REDIRECT],
      token_endpoint_auth_method: "none",
    },
  });
  expect(registered.status()).toBe(201);
  const { client_id } = await registered.json();

  const verifier = randomBytes(32).toString("base64url");
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  const query = new URLSearchParams({
    response_type: "code",
    client_id,
    redirect_uri: REDIRECT,
    code_challenge: challenge,
    code_challenge_method: "S256",
    state: "push",
    resource: `${base}/api`,
  });
  const consent = await api.get(`/oauth/authorize?${query}`);
  expect(consent.status()).toBe(200);
  const allowed = await api.post("/oauth/authorize", {
    form: {
      ...Object.fromEntries(query),
      csrf: csrfFrom(await consent.text()),
      decision: "allow",
    },
    maxRedirects: 0,
  });
  expect(allowed.status()).toBe(303);
  const code = new URL(allowed.headers()["location"]).searchParams.get("code");

  const token = await api.post("/oauth/token", {
    form: {
      grant_type: "authorization_code",
      client_id,
      code: code!,
      code_verifier: verifier,
      redirect_uri: REDIRECT,
      resource: `${base}/api`,
    },
  });
  expect(token.status()).toBe(200);
  return (await token.json()).access_token;
}

test.describe("push notifications", () => {
  let stub: Awaited<ReturnType<typeof apnsStub>>;
  let workDir: string;
  let envFile: string;
  let base: string;
  let logFile: string;

  test.beforeAll(async () => {
    stub = await apnsStub();
    workDir = mkdtempSync(path.join(tmpdir(), "folio-push-e2e-"));
    const keyFile = path.join(workDir, "apns.p8");
    writeFileSync(keyFile, KEY);
    const extra = path.join(workDir, "extra.env");
    writeFileSync(
      extra,
      [
        "FOLIO_PUSH__TRANSPORT=apns",
        "FOLIO_PUSH__APNS__TEAM_ID=TEAM123456",
        "FOLIO_PUSH__APNS__KEY_ID=KEY1234567",
        `FOLIO_PUSH__APNS__PRIVATE_KEY_FILE=${keyFile}`,
        `FOLIO_PUSH__APNS__BASE_URL=${stub.url}`,
        "FOLIO_PUSH__QUIET_WINDOW_SECONDS=1",
        "FOLIO_OCR__PROVIDER=none",
      ].join("\n"),
    );
    envFile = execFileSync(path.join(scripts, "up.sh"), {
      encoding: "utf8",
      env: {
        ...process.env,
        FOLIO_VERIFY_AUTH_REQUIRED: "true",
        FOLIO_VERIFY_EXTRA_ENV: extra,
      },
    }).trim();
    const vars = Object.fromEntries(
      readFileSync(envFile, "utf8")
        .split("\n")
        .filter(Boolean)
        .map((line) => line.split(/=(.*)/s).slice(0, 2)),
    );
    base = vars.BASE_URL;
    logFile = path.join(vars.VERIFY_DIR, "server.log");
  });

  test.afterAll(() => {
    if (envFile) execFileSync(path.join(scripts, "down.sh"), [envFile]);
    stub?.close();
    if (workDir) rmSync(workDir, { recursive: true, force: true });
  });

  test("an upload from a signed-in app reaches APNs without its title", async () => {
    test.setTimeout(90_000);
    const setupToken = readFileSync(logFile, "utf8").match(
      /setup\?token=([A-Za-z0-9_-]+)/,
    )![1];
    const browserless = await playwrightRequest.newContext({ baseURL: base });
    const accessToken = await signInApp(browserless, base, setupToken);
    const app = await playwrightRequest.newContext({
      baseURL: base,
      extraHTTPHeaders: { Authorization: `Bearer ${accessToken}` },
    });

    const registered = await app.put("/api/devices/e2e-phone", {
      data: {
        platform: "ios",
        environment: "production",
        locale: "en",
        alert_token: DEVICE_TOKEN,
      },
    });
    expect(registered.status()).toBe(201);
    expect((await registered.json()).push_available).toBe(true);

    const title = `push-e2e-secret-${Date.now()}`;
    const uploaded = await app.post("/api/documents/upload", {
      multipart: {
        title,
        file: {
          name: "sample.pdf",
          mimeType: "application/pdf",
          buffer: readFileSync(samplePdf),
        },
      },
    });
    expect(uploaded.ok()).toBeTruthy();

    await expect
      .poll(() => stub.pushes.length, { timeout: 60_000 })
      .toBeGreaterThan(0);
    const [push] = stub.pushes;
    expect(push.path).toBe(`/3/device/${DEVICE_TOKEN}`);
    expect(push.headers["apns-push-type"]).toBe("alert");
    expect(push.headers["apns-topic"]).toBe("com.cedricziel.folio");
    expect(String(push.headers["authorization"])).toMatch(/^bearer ey/i);
    const payload = JSON.parse(push.body);
    expect(payload.aps.alert.body).toMatch(
      /^1 document (needs review|is ready|failed processing)$/,
    );
    expect(push.body).not.toContain(title);

    const devices = await app.get("/api/devices");
    expect((await devices.json()).devices).toHaveLength(1);
    await browserless.dispose();
    await app.dispose();
  });
});
