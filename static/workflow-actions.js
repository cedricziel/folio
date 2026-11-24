export class WorkflowActionEditor {
  constructor(root) {
    this.root = root;
    this.rows = root.querySelector("[data-action-rows]");
    this.kind = root.querySelector("[data-action-kind]");
    this.addControls = root.querySelector("[data-action-add-controls]");
    this.payload = root.querySelector("[data-action-payload]");
    this.savedPayload = this.payload.value;
    this.empty = root.querySelector("[data-action-empty]");
    this.controller = new AbortController();
    const { signal } = this.controller;

    this.rows.replaceChildren();
    for (const draft of JSON.parse(this.savedPayload)) this.add(draft);
    root.addEventListener("click", (event) => this.changeRows(event), { signal });
    this.payload.form.addEventListener("submit", () => this.serialize(), { signal });
    document.addEventListener("turbo:before-cache", () => this.clearBeforeCache(), { signal });
    this.payload.disabled = false;
    root.hidden = false;
  }

  disconnect() {
    this.controller.abort();
  }

  clearBeforeCache() {
    this.rows.replaceChildren();
    this.payload.value = this.savedPayload;
    this.payload.disabled = true;
    this.root.hidden = true;
  }

  template(name) {
    return this.root.querySelector(`template[data-action-template="${name}"]`);
  }

  add(draft) {
    const definition = this.template(draft.kind);
    if (!definition) return;
    const fields = definition.dataset.actionFieldTemplate
      ? this.template(definition.dataset.actionFieldTemplate)
      : definition;
    const row = this.template("row").content.firstElementChild.cloneNode(true);
    row.dataset.kind = draft.kind;
    row.dataset.existing = draft.existing ?? "";
    row.dataset.hasSecret = Boolean(draft.has_secret);
    row.querySelector("legend").textContent = definition.dataset.label;
    row.querySelector("[data-action-fields]").append(fields.content.cloneNode(true));
    const value = row.querySelector("[data-action-value]");
    if (value) {
      if (value instanceof HTMLSelectElement && draft.value && !Array.from(value.options).some((option) => option.value === draft.value)) {
        value.add(new Option("Unavailable choice", draft.value));
      }
      value.value = draft.value ?? "";
    }
    for (const id of draft.tag_ids ?? []) {
      let checkbox = Array.from(row.querySelectorAll("[data-action-tag]")).find((tag) => tag.value === id);
      if (!checkbox) {
        const label = document.createElement("label");
        label.className = "check";
        checkbox = document.createElement("input");
        checkbox.type = "checkbox";
        checkbox.dataset.actionTag = "";
        checkbox.value = id;
        label.append(checkbox, " Unavailable tag");
        row.querySelector("[data-action-tags]").append(label);
      }
      checkbox.checked = true;
    }
    if (draft.kind === "webhook") {
      row.querySelector("[data-action-headers]").value = draft.headers ?? "";
      row.querySelector("[data-action-clear-secret]").checked = Boolean(draft.remove_secret);
      row.querySelector("[data-action-secret-removal]").hidden = !draft.has_secret;
      if (draft.has_secret) {
        row.querySelector("[data-action-secret]").placeholder = "Leave empty to keep the saved secret";
      }
    }
    this.rows.append(row);
    return row;
  }

  changeRows(event) {
    const button = event.target.closest("[data-action-command]");
    if (!button || button.disabled) return;
    const row = button.closest("[data-action-row]");
    const command = button.dataset.actionCommand;
    switch (command) {
      case "add":
        this.add({ kind: this.kind.value })?.querySelector("input, select")?.focus();
        break;
      case "up":
        row.previousElementSibling.before(row);
        break;
      case "down":
        row.nextElementSibling.after(row);
        break;
      case "remove":
        const focus = row.nextElementSibling ?? row.previousElementSibling;
        row.remove();
        const fallback = this.ingest ? this.payload.form.querySelector('select[name="trigger"]') : this.kind;
        (focus?.querySelector('[data-action-command="remove"]') ?? fallback).focus();
        break;
    }
    this.sync(this.documentActions, this.ingest);
    if (command === "up" || command === "down") {
      (button.disabled ? row.querySelector('[data-action-command="remove"]') : button).focus();
    }
  }

  sync(documentActions, ingest) {
    this.documentActions = documentActions;
    this.ingest = ingest;
    this.addControls.hidden = ingest;
    for (const option of this.kind.options) {
      option.hidden = !documentActions && option.hasAttribute("data-document-action");
      option.disabled = option.hidden;
    }
    if (this.kind.selectedOptions[0]?.disabled) this.kind.value = "webhook";
    this.empty.hidden = ingest || this.rows.childElementCount > 0;
    this.rows.querySelectorAll("[data-action-row]").forEach((row) => {
      row.querySelector('[data-action-command="up"]').disabled = !row.previousElementSibling;
      row.querySelector('[data-action-command="down"]').disabled = !row.nextElementSibling;
      const incompatible = ingest || (!documentActions && row.dataset.kind !== "webhook");
      row.querySelector("[data-action-incompatible]").hidden = !incompatible;
      row.querySelectorAll("[data-action-fields] input, [data-action-fields] select, [data-action-fields] textarea")
        .forEach((control) => { control.disabled = incompatible; });
    });
    this.root.hidden = ingest && this.rows.childElementCount === 0;
    return !this.root.hidden;
  }

  serialize() {
    this.payload.value = JSON.stringify(Array.from(this.rows.children, (row) => ({
      kind: row.dataset.kind,
      existing: row.dataset.existing === "" ? null : Number(row.dataset.existing),
      value: row.querySelector("[data-action-value]")?.value ?? "",
      tag_ids: Array.from(row.querySelectorAll("[data-action-tag]:checked"), (tag) => tag.value),
      headers: row.querySelector("[data-action-headers]")?.value ?? "",
      secret: row.querySelector("[data-action-secret]")?.value ?? "",
      remove_secret: row.querySelector("[data-action-clear-secret]")?.checked ?? false,
      has_secret: row.dataset.hasSecret === "true",
    })));
  }
}
