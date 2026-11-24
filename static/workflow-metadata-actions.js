const pageSize = 100;

export function metadataCatalog(root, signal) {
  const fieldsURL = root.dataset.metadataFieldsUrl;

  async function json(url) {
    const response = await fetch(url, { signal });
    if (!response.ok) throw new Error("Could not load choices");
    return response.json();
  }

  async function pages(path) {
    const url = new URL(path, document.baseURI);
    url.searchParams.set("limit", String(pageSize));
    const choices = [];
    for (;;) {
      const page = await json(url);
      choices.push(...page);
      if (page.length < pageSize) return choices;
      url.searchParams.set("after", page.at(-1).id);
    }
  }

  let fields;
  return {
    root,
    signal,
    fields() {
      fields ??= pages(fieldsURL).catch((error) => {
        fields = undefined;
        throw error;
      });
      return fields;
    },
    async options(field) {
      const choices = await pages(`${fieldsURL}/${encodeURIComponent(field)}/options`);
      return choices.sort((a, b) => a.position - b.position || a.id.localeCompare(b.id));
    },
    async documents(query) {
      const url = new URL(root.dataset.metadataDocumentsUrl, document.baseURI);
      url.searchParams.set("q", query);
      url.searchParams.set("limit", "50");
      const { results } = await json(url);
      return results.map(({ document }) => ({ id: document.id, label: document.title }));
    },
  };
}

export class WorkflowMetadataAction {
  constructor(row, draft, catalog) {
    this.catalog = catalog;
    this.field = row.querySelector("[data-metadata-field]");
    this.fields = row.querySelector("[data-metadata-value]");
    this.status = row.querySelector("[data-metadata-status]");
    this.retry = row.querySelector("[data-metadata-retry]");
    this.clearing = draft.kind === "clear_metadata";
    this.generation = 0;
    if (draft.value) {
      const saved = new Option("Unavailable custom field", draft.value, false, true);
      saved.dataset.kind = draft.metadata_value?.kind ?? "";
      this.field.add(saved);
    }
    this.field.addEventListener("change", () => this.renderValue());
    this.renderValue(draft.metadata_value);
    this.loadFields();
  }

  notice(message, retry) {
    this.status.hidden = !message;
    this.status.textContent = message;
    this.retry.hidden = !retry;
    this.retry.onclick = retry ?? null;
  }

  async loadFields() {
    this.notice("Loading custom fields…");
    try {
      const fields = await this.catalog.fields();
      if (!this.field.isConnected || this.catalog.signal.aborted) return;
      const selected = this.field.selectedOptions[0];
      const choices = fields
        .filter((field) => !this.clearing || !field.required)
        .sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id))
        .map((field) => {
          const option = new Option(field.name, field.id);
          option.dataset.kind = field.kind;
          option.dataset.required = String(field.required);
          return option;
        });
      this.field.replaceChildren(new Option("Choose a custom field", ""), ...choices);
      if (selected.value && !choices.some((choice) => choice.value === selected.value)) {
        this.field.add(selected);
      }
      this.field.value = selected.value;
      const field = this.field.selectedOptions[0];
      if (field.value && !this.kind) this.renderValue();
      const changedKind = Boolean(this.kind && field.dataset.kind !== this.kind);
      const input = this.fields.querySelector("[data-metadata-input-value]");
      if (input && ["text", "long_text"].includes(this.kind)) input.required = field.dataset.required === "true";
      if (changedKind) {
        this.notice("The field type changed. Choose a different field or remove this action.");
      } else if (!this.fields.querySelector("[data-metadata-option-loading]")) {
        this.notice("");
      }
    } catch {
      if (!this.catalog.signal.aborted) this.notice("Could not load custom fields.", () => this.loadFields());
    }
  }

  template(attribute) {
    return this.catalog.root.querySelector(attribute).content.cloneNode(true);
  }

  renderValue(saved) {
    const generation = ++this.generation;
    this.fields.replaceChildren();
    this.notice("");
    const field = this.field.selectedOptions[0];
    this.kind = field?.dataset.kind;
    if (this.clearing || !this.kind) return;

    const reference = ["select", "document_link"].includes(this.kind);
    const type = reference ? "reference" : ["boolean", "long_text"].includes(this.kind) ? this.kind : "text";
    const label = document.createElement("label");
    label.append(this.kind === "money" ? "Amount" : "Value", this.template(`[data-metadata-input="${type}"]`));
    const input = label.querySelector("[data-metadata-input-value]");
    if (input instanceof HTMLInputElement) {
      if (["date", "url"].includes(this.kind)) input.type = this.kind;
      if (["decimal", "money"].includes(this.kind)) input.inputMode = "decimal";
      input.required = this.kind !== "text" || field.dataset.required === "true";
    } else if (this.kind === "long_text") {
      input.required = field.dataset.required === "true";
    }

    const value = saved?.kind === this.kind ? saved.value : undefined;
    if (reference && value) input.add(new Option("Saved value", value));
    input.value = value?.amount ?? (typeof value === "object" ? "" : String(value ?? ""));
    if (this.kind === "money") {
      this.fields.append(this.template("[data-metadata-currency]"));
      this.fields.querySelector("[data-metadata-currency-value]").value = value?.currency ?? "";
    }
    this.fields.prepend(label);
    if (this.kind === "select") this.loadOptions(input, field.value, generation);
    if (this.kind === "document_link") this.addDocumentSearch(input, generation);
  }

  replaceChoices(input, choices) {
    const selected = input.selectedOptions[0];
    input.replaceChildren(
      new Option("Choose a value", ""),
      ...choices.map((choice) => new Option(choice.label, choice.id)),
    );
    if (selected?.value && !choices.some((choice) => choice.id === selected.value)) input.add(selected);
    input.value = selected?.value ?? "";
  }

  current(input, generation) {
    return input.isConnected && this.generation === generation && !this.catalog.signal.aborted;
  }

  async loadOptions(input, field, generation) {
    input.dataset.metadataOptionLoading = "";
    this.notice("Loading options…");
    try {
      const options = await this.catalog.options(field);
      if (!this.current(input, generation)) return;
      this.replaceChoices(input, options);
      this.notice(options.length === 0 ? "This field has no available options." : "");
    } catch {
      if (this.current(input, generation)) this.notice("Could not load options.", () => this.loadOptions(input, field, generation));
    } finally {
      delete input.dataset.metadataOptionLoading;
    }
  }

  addDocumentSearch(input, generation) {
    this.fields.prepend(this.template("[data-metadata-document-search]"));
    const search = this.fields.querySelector("[data-metadata-query]");
    const button = this.fields.querySelector("[data-metadata-search]");
    const find = async () => {
      const query = search.value.trim();
      if (!query || button.disabled || button.matches(":disabled")) return;
      button.disabled = true;
      this.notice("Searching documents…");
      try {
        const results = await this.catalog.documents(query);
        if (!this.current(input, generation)) return;
        this.replaceChoices(input, results);
        this.notice(results.length === 0 ? "No matching documents." : "");
      } catch {
        if (this.current(input, generation)) this.notice("Could not search documents.");
      } finally {
        button.disabled = false;
      }
    };
    button.addEventListener("click", find);
    search.addEventListener("keydown", (event) => {
      if (event.key !== "Enter") return;
      event.preventDefault();
      find();
    });
  }

  value() {
    const input = this.fields.querySelector("[data-metadata-input-value]");
    if (!input || !input.validity.valid) return;
    let value = input.value;
    if (this.kind === "boolean") value = value === "true";
    if (this.kind === "money") {
      const currency = this.fields.querySelector("[data-metadata-currency-value]");
      if (!currency.validity.valid) return;
      value = { amount: value, currency: currency.value };
    }
    return { kind: this.kind, value };
  }
}
