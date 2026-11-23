const operatorLabels = {
  all: "All of these",
  any: "Any of these",
  none: "None of these",
};
const emptyGroupText = {
  all: "An empty group always matches.",
  any: "An empty group never matches.",
  none: "An empty group always matches.",
};
const fieldTargets = ["documentFilter", "eventFilter", "ingestFilter", "metadataFilter", "processingFilter"];
const scalarFields = {
  mime_type: "mime_type",
  filename_glob: "filename_glob",
  tag_id: "tag_id",
  correspondent_id: "correspondent_id",
  document_type_id: "document_type_id",
  source_kind: "source.kind",
  folder_path_glob: "source.folder_path_glob",
  email_attachment_glob: "source.email_attachment_glob",
  email_mailbox_glob: "source.email_mailbox_glob",
  event_subject_id: "event.subject_id",
  ingest_source_id: "event.ingest.source_id",
  ingest_source_kind: "event.ingest.source_kind",
  ingest_reason: "event.ingest.reason_code",
  ingest_retryable: "event.ingest.retryable",
  metadata_field_id: "event.metadata.field_id",
  metadata_kind: "event.metadata.kind",
  metadata_state: "event.metadata.state",
  processing_stage: "event.processing.stage",
  processing_outcome: "event.processing.outcome",
  processing_run_id: "event.processing.run_id",
  processing_revision_id: "event.processing.revision_id",
  processing_attempt: "event.processing.attempt",
  processing_reason_code: "event.processing.reason_code",
  processing_retry_at: "event.processing.retry_at",
  processing_has_text: "event.processing.has_text",
};
const emailFields = ["email_from", "email_to", "email_subject"];
const metadataSides = ["before", "after"];

function read(object, path) {
  return path.split(".").reduce((value, key) => value?.[key], object);
}

function write(object, path, value) {
  const keys = path.split(".");
  const leaf = keys.pop();
  const parents = [];
  for (const key of keys) {
    parents.push([object, key]);
    if (object[key] == null || typeof object[key] !== "object" || Array.isArray(object[key])) object[key] = {};
    object = object[key];
  }
  if (value === undefined) delete object[leaf];
  else object[leaf] = value;
  for (const [parent, key] of parents.reverse()) {
    if (!Object.keys(parent[key]).length) delete parent[key];
  }
}

function clearEmpty(value) {
  return value === "" ? undefined : value;
}

function setControl(control, value) {
  if (!control) return;
  if (control.type === "checkbox") {
    control.checked = Boolean(value);
  } else {
    const text = value == null ? "" : String(value);
    if (control instanceof HTMLSelectElement && text && !Array.from(control.options).some((option) => option.value === text)) {
      control.add(new Option("Unavailable choice (" + text + ")", text));
    }
    control.value = text;
  }
}

function controlValue(control) {
  return control?.type === "checkbox" ? control.checked : control?.value ?? "";
}

function metadataPredicate(kind, text, currency) {
  if (kind === "money") return { kind, value: { amount: text, currency } };
  if (kind === "boolean") {
    const value = text.trim().toLowerCase();
    if (value === "true" || value === "yes") return { kind, value: true };
    if (value === "false" || value === "no") return { kind, value: false };
  }
  return { kind, value: text };
}

function prepareBranch(fields, draft) {
  const put = (name, value) => setControl(fields.get(name), value);
  for (const [name, path] of Object.entries(scalarFields)) put(name, read(draft, path));
  for (const name of emailFields) {
    const match = draft.source?.[name];
    const [mode, value] = Object.entries(match ?? {})[0] ?? ["glob", ""];
    put(name + "_mode", mode);
    put(name, value);
  }
  put("event_actor", draft.event?.actor?.type);
  put("event_user_id", draft.event?.actor?.user_id);
  for (const side of metadataSides) {
    const predicate = draft.event?.metadata?.[side];
    put("metadata_" + side + "_enabled", predicate != null);
    put("metadata_" + side, predicate?.kind === "money" ? predicate.value?.amount : predicate?.value);
    put("metadata_" + side + "_currency", predicate?.value?.currency);
  }
}

function branchConditions(draft, initial, fields) {
  const conditions = structuredClone(draft);
  const value = (name) => controlValue(fields.get(name));
  const changed = (...names) => names.some((name) => fields.has(name) && initial.get(name) !== value(name));
  for (const [name, path] of Object.entries(scalarFields)) {
    if (!changed(name)) continue;
    const raw = value(name).trim();
    let parsed = clearEmpty(raw);
    if (raw && (name === "ingest_retryable" || name === "processing_has_text")) {
      parsed = raw === "true";
    } else if (name === "processing_attempt" && /^[+]?\d+$/.test(raw) && Number(raw) <= 4294967295) {
      parsed = Number(raw);
    }
    write(conditions, path, parsed);
  }
  for (const name of emailFields) {
    if (!changed(name, name + "_mode")) continue;
    const text = value(name).trim();
    write(conditions, "source." + name, text ? { [value(name + "_mode")]: text } : undefined);
  }
  if (changed("event_actor", "event_user_id")) {
    const type = value("event_actor");
    const actor = type === "user" ? { type, user_id: value("event_user_id").trim() } : type ? { type } : undefined;
    write(conditions, "event.actor", actor);
  }
  for (const side of metadataSides) {
    const field = "metadata_" + side;
    const enabled = field + "_enabled";
    const currency = field + "_currency";
    const path = "event.metadata." + side;
    if (!changed("metadata_kind", field, enabled, currency)) continue;
    const kind = value("metadata_kind") || read(draft, path)?.kind || "";
    const predicate = value(enabled) ? metadataPredicate(kind, value(field), value(currency)) : undefined;
    write(conditions, path, predicate);
  }
  return conditions;
}

export function syncProcessingFields(fieldset, supported) {
  for (const row of fieldset.querySelectorAll("[data-processing-key]")) {
    const control = row.querySelector("input, select");
    row.hidden = !supported.includes(row.dataset.processingKey) && control.value === "";
    control.disabled = row.hidden;
  }
  fieldset.hidden = !fieldset.querySelector("[data-processing-key]:not([hidden])");
  fieldset.disabled = fieldset.hidden;
}

export class WorkflowConditionEditor {
  constructor(root) {
    this.root = root;
    this.payload = root.querySelector("[data-condition-payload]");
    this.form = this.payload.form;
    this.templates = [...this.form.querySelectorAll("fieldset[data-workflow-form-target]")]
      .filter((fieldset) => fieldTargets.includes(fieldset.dataset.workflowFormTarget));
    this.branchState = new WeakMap();
    this.branchCount = 0;
    this.controller = new AbortController();
    const { signal } = this.controller;
    this.initialPayload = this.payload.value;
    this.reset();
    const draft = JSON.parse(this.initialPayload);
    this.addNodeControls(root, draft);
    this.initialGroups = JSON.stringify(this.groups(root));
    root.addEventListener("click", (event) => this.changeGroups(event), { signal });
    root.addEventListener("input", () => this.sync(this.visibility), { signal });
    root.addEventListener("change", () => this.sync(this.visibility), { signal });
    this.form.addEventListener("submit", () => this.serialize(), { signal });
    document.addEventListener("turbo:before-cache", () => this.reset(), { signal });
    root.hidden = false;
  }

  disconnect() {
    this.controller.abort();
  }

  reset() {
    this.container(this.root).replaceChildren();
    this.root.querySelector(":scope > [data-condition-add]")?.remove();
    this.payload.value = this.initialPayload;
    this.payload.disabled = true;
    this.root.hidden = true;
  }

  clone(name) {
    return this.root.querySelector('template[data-condition-template="' + name + '"]').content.firstElementChild.cloneNode(true);
  }

  container(node) {
    return node.querySelector(":scope > [data-condition-groups]");
  }

  addNodeControls(node, draft) {
    const container = this.container(node);
    for (const operator of Object.keys(operatorLabels)) {
      if (Array.isArray(draft?.[operator])) this.addGroup(container, operator, draft[operator]);
    }
    container.after(this.clone("add"));
    this.refreshAdd(node);
  }

  addGroup(container, operator, branches = []) {
    const group = this.clone("group");
    group.dataset.operator = operator;
    group.querySelector("legend").textContent = operatorLabels[operator];
    group.querySelector("[data-condition-empty]").textContent = emptyGroupText[operator];
    container.append(group);
    for (const branch of branches) this.addBranch(group, branch);
    this.refreshGroup(group);
    return group;
  }

  addBranch(group, draft) {
    const branch = this.clone("branch");
    const source = draft && typeof draft === "object" && !Array.isArray(draft) ? draft : {};
    const prefix = "workflow-condition-" + ++this.branchCount + "-";
    const fields = new Map();
    const controls = branch.querySelector("[data-condition-fields]");
    if (draft && Object.keys(source).every((key) => Object.hasOwn(operatorLabels, key))) controls.open = false;
    for (const template of this.templates) {
      const fieldset = template.cloneNode(true);
      for (const item of [fieldset, ...fieldset.querySelectorAll("[data-workflow-form-target]")]) {
        item.dataset.branchTarget = item.dataset.workflowFormTarget;
        delete item.dataset.workflowFormTarget;
      }
      for (const item of fieldset.querySelectorAll("[data-action]")) item.removeAttribute("data-action");
      for (const item of fieldset.querySelectorAll("[id]")) item.id = prefix + item.id;
      for (const label of fieldset.querySelectorAll("label[for]")) label.htmlFor = prefix + label.htmlFor;
      for (const control of fieldset.querySelectorAll("[name]")) {
        fields.set(control.name, control);
        control.removeAttribute("name");
      }
      controls.append(fieldset);
    }
    prepareBranch(fields, source);
    const initial = new Map(Array.from(fields, ([name, control]) => [name, controlValue(control)]));
    this.branchState.set(branch, { source, fields, initial });
    group.querySelector(":scope > [data-condition-branches]").append(branch);
    this.addNodeControls(branch, source);
    return branch;
  }

  refreshGroup(group) {
    const count = group.querySelector(":scope > [data-condition-branches]").childElementCount;
    group.querySelector(":scope > [data-condition-empty]").hidden = count !== 0;
  }

  refreshAdd(node) {
    const controls = node.querySelector(":scope > [data-condition-add]");
    const select = controls.querySelector("select");
    const used = new Set(Array.from(this.container(node).children, (group) => group.dataset.operator));
    for (const option of select.options) option.disabled = used.has(option.value);
    if (select.selectedOptions[0]?.disabled) {
      select.value = Array.from(select.options).find((option) => !option.disabled)?.value ?? "";
    }
    controls.hidden = used.size === select.length;
  }

  changeGroups(event) {
    const button = event.target.closest("[data-condition-command]");
    if (!button) return;
    const group = button.closest("[data-condition-group]");
    const branch = button.closest("[data-condition-branch]");
    const node = branch ?? this.root;
    switch (button.dataset.conditionCommand) {
      case "add-group": {
        const operator = node.querySelector(":scope > [data-condition-add] select").value;
        const created = this.addGroup(this.container(node), operator);
        this.refreshAdd(node);
        created.querySelector('[data-condition-command="add-branch"]').focus();
        break;
      }
      case "remove-group": {
        const owner = group.parentElement.parentElement;
        group.remove();
        this.refreshAdd(owner);
        owner.querySelector(":scope > [data-condition-add] select").focus();
        break;
      }
      case "add-branch": {
        const child = this.addBranch(group);
        this.refreshGroup(group);
        this.sync(this.visibility);
        child.querySelector("summary").focus();
        break;
      }
      case "remove-branch": {
        const owner = branch.parentElement.parentElement;
        branch.remove();
        this.refreshGroup(owner);
        owner.querySelector(':scope > .condition-controls [data-condition-command="add-branch"]').focus();
        break;
      }
    }
  }

  sync(visibility) {
    this.visibility = visibility;
    const { trigger, ingest, outcome, health, metadata, metadataValues, documentFilters, processingKeys = [] } = visibility;
    for (const branch of this.root.querySelectorAll("[data-condition-branch]")) {
      const { source, fields, initial } = this.branchState.get(branch);
      const target = branch.querySelector(":scope > [data-condition-fields]");
      const value = (name) => controlValue(fields.get(name));
      const draft = branchConditions(source, initial, fields);
      const show = (name, visible) => {
        for (const element of target.querySelectorAll('[data-branch-target="' + name + '"]')) {
          element.hidden = !visible;
          if (element instanceof HTMLFieldSetElement) element.disabled = !visible;
        }
      };
      const documentFields = ["mime_type", "filename_glob", "tag_id", "correspondent_id", "document_type_id"].some((name) => draft[name] != null);
      show("documentFilter", documentFilters || documentFields || Boolean(draft.source));
      show("eventFilter", !ingest || Boolean(draft.event?.actor || draft.event?.subject_id));
      show("ingestFilter", outcome || health || Boolean(draft.event?.ingest));
      show("ingestReasonLabel", trigger !== "source.recovered" || Boolean(draft.event?.ingest?.reason_code));
      show("ingestReason", trigger !== "source.recovered" || Boolean(draft.event?.ingest?.reason_code));
      show("ingestRetryLabel", outcome || draft.event?.ingest?.retryable != null);
      show("ingestRetry", outcome || draft.event?.ingest?.retryable != null);
      show("metadataFilter", metadata || Boolean(draft.event?.metadata));
      syncProcessingFields(target.querySelector('[data-branch-target="processingFilter"]'), processingKeys);
      show("metadataValues", metadataValues || Boolean(draft.event?.metadata?.before || draft.event?.metadata?.after || draft.event?.metadata?.state));
      show("metadataCurrency", value("metadata_kind") === "money" || draft.event?.metadata?.before?.kind === "money" || draft.event?.metadata?.after?.kind === "money");
      show("actorUser", value("event_actor") === "user");
      show("folder", value("source_kind") === "folder" || Boolean(value("folder_path_glob")));
      show("email", value("source_kind") === "email" || [...emailFields, "email_attachment_glob", "email_mailbox_glob"].some(value));
    }
  }

  groups(node) {
    const groups = {};
    for (const group of this.container(node).children) {
      const branches = group.querySelector(":scope > [data-condition-branches]").children;
      groups[group.dataset.operator] = Array.from(branches, (branch) => {
        const { source, fields, initial } = this.branchState.get(branch);
        const conditions = branchConditions(source, initial, fields);
        for (const operator of Object.keys(operatorLabels)) delete conditions[operator];
        return Object.assign(conditions, this.groups(branch));
      });
    }
    return groups;
  }

  serialize() {
    const groups = JSON.stringify(this.groups(this.root));
    this.payload.value = groups;
    this.payload.disabled = groups === this.initialGroups && !this.payload.hasAttribute("data-condition-draft");
  }
}
