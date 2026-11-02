import { expect, test } from "@playwright/test";
import { TourInstance, tour } from "./tour";

const instance = new TourInstance();
test.beforeAll(() => instance.start());
test.afterAll(() => instance.stop());

test("unavailable passkey autofill leaves password sign-in usable", async ({
  browser,
}) => {
  await instance.ready(browser, { seeded: false });
  const context = await browser.newContext({ baseURL: instance.baseURL });
  await context.addInitScript(() => {
    PublicKeyCredential.isConditionalMediationAvailable = async () => true;
    navigator.credentials.get = async () => {
      document.documentElement.dataset.passkeyAttempted = "true";
      throw new DOMException("Passkeys are unavailable.", "NotSupportedError");
    };
  });
  const page = await context.newPage();
  try {
    await page.goto("/login");
    await expect(page.locator("html")).toHaveAttribute(
      "data-passkey-attempted",
      "true",
    );
    await expect(page.locator("[data-passkey-target='error']")).toBeHidden();
    await page
      .getByRole("button", { name: "Sign in with a passkey", exact: true })
      .click();
    await expect(page.getByRole("alert")).toHaveText(
      "Passkeys are unavailable.",
    );
    await tour.login(page);
    await expect(page).toHaveURL(instance.baseURL + "/");
  } finally {
    await context.close();
  }
});

test("a resident passkey signs in without a username", async ({ browser }) => {
  await instance.ready(browser, { seeded: false });
  const context = await browser.newContext({ baseURL: instance.baseURL });
  const page = await context.newPage();
  const cdp = await context.newCDPSession(page);
  await cdp.send("WebAuthn.enable");
  const { authenticatorId } = await cdp.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });
  try {
    await tour.login(page);
    await page.goto("/settings/security");
    await page.getByLabel("Name", { exact: true }).fill("Browser passkey");
    await page.locator("#passkey_password").fill(tour.credentials.password);
    await page.getByRole("button", { name: "Add a passkey", exact: true }).click();
    await expect(page.getByRole("button", { name: "Rename", exact: true })).toBeVisible();
    await cdp.send("WebAuthn.setAutomaticPresenceSimulation", { authenticatorId, enabled: false });
    await page.getByRole("button", { name: "Sign out", exact: true }).click();
    await expect(page).toHaveURL(/\/login/);
    await page.getByRole("button", { name: "Sign in with a passkey", exact: true }).click();
    await cdp.send("WebAuthn.setAutomaticPresenceSimulation", { authenticatorId, enabled: true });
    await expect(page).toHaveURL(instance.baseURL + "/");
  } finally {
    await context.close();
  }
});
