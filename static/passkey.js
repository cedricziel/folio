// Passkey sign-in and registration with navigator.credentials.
import { Application, Controller } from "@hotwired/stimulus";

const toBuffer = (value) => {
  const padded = value.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(value.length / 4) * 4, "=");
  return Uint8Array.from(atob(padded), (c) => c.charCodeAt(0)).buffer;
};

const toBase64Url = (buffer) =>
  btoa(String.fromCharCode(...new Uint8Array(buffer)))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");

const decodeIds = (list) => (list || []).map((item) => ({ ...item, id: toBuffer(item.id) }));

async function post(url, body, csrf) {
  const headers = { "Content-Type": "application/json" };
  if (csrf) headers["X-CSRF"] = csrf;
  const response = await fetch(url, {
    method: "POST",
    headers,
    credentials: "same-origin",
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error || "That did not work. Try again.");
  return data;
}

class PasskeyController extends Controller {
  static targets = ["button", "error", "name"];
  static values = { next: String, csrf: String, mode: String };

  connect() {
    this.supported = Boolean(window.PublicKeyCredential && navigator.credentials);
    if (!this.supported) {
      this.hideAll();
      return;
    }
    if (this.modeValue === "login") this.autofill();
  }

  disconnect() {
    this.abort?.abort();
  }

  hideAll() {
    this.element.hidden = true;
  }

  fail(error) {
    if (error?.name === "AbortError") return;
    const message =
      error?.name === "NotAllowedError" ? "The passkey prompt was closed or timed out." : error.message;
    if (this.hasErrorTarget) {
      this.errorTarget.textContent = message;
      this.errorTarget.hidden = false;
    }
  }

  async assertion(mediation) {
    const options = await post("/auth/passkey/start");
    const publicKey = options.publicKey;
    publicKey.challenge = toBuffer(publicKey.challenge);
    publicKey.allowCredentials = decodeIds(publicKey.allowCredentials);
    this.abort?.abort();
    this.abort = new AbortController();
    const credential = await navigator.credentials.get({ publicKey, mediation, signal: this.abort.signal });
    const response = credential.response;
    const result = await post("/auth/passkey/finish", {
      credential: {
        id: credential.id,
        rawId: toBase64Url(credential.rawId),
        type: credential.type,
        extensions: credential.getClientExtensionResults(),
        response: {
          authenticatorData: toBase64Url(response.authenticatorData),
          clientDataJSON: toBase64Url(response.clientDataJSON),
          signature: toBase64Url(response.signature),
          userHandle: response.userHandle ? toBase64Url(response.userHandle) : null,
        },
      },
      next: this.nextValue,
    });
    window.location.assign(result.redirect);
  }

  async autofill() {
    const available = await PublicKeyCredential.isConditionalMediationAvailable?.();
    if (!available || !document.querySelector("input[autocomplete~='webauthn']")) return;
    try {
      await this.assertion("conditional");
    } catch (error) {
      this.fail(error);
    }
  }

  async login() {
    if (this.hasErrorTarget) this.errorTarget.hidden = true;
    try {
      await this.assertion("required");
    } catch (error) {
      this.fail(error);
    }
  }

  async register() {
    if (this.hasErrorTarget) this.errorTarget.hidden = true;
    try {
      const url = "/settings/security/passkeys/register";
      const options = await post(`${url}/start`, undefined, this.csrfValue);
      const publicKey = options.publicKey;
      publicKey.challenge = toBuffer(publicKey.challenge);
      publicKey.user.id = toBuffer(publicKey.user.id);
      publicKey.excludeCredentials = decodeIds(publicKey.excludeCredentials);
      const credential = await navigator.credentials.create({ publicKey });
      const response = credential.response;
      await post(
        `${url}/finish`,
        {
          name: this.hasNameTarget ? this.nameTarget.value : "",
          credential: {
            id: credential.id,
            rawId: toBase64Url(credential.rawId),
            type: credential.type,
            extensions: credential.getClientExtensionResults(),
            response: {
              attestationObject: toBase64Url(response.attestationObject),
              clientDataJSON: toBase64Url(response.clientDataJSON),
              transports: response.getTransports ? response.getTransports() : [],
            },
          },
        },
        this.csrfValue,
      );
      window.location.assign("/settings/security");
    } catch (error) {
      this.fail(error);
    }
  }
}

const application = window.Stimulus || (window.Stimulus = Application.start());
application.register("passkey", PasskeyController);
