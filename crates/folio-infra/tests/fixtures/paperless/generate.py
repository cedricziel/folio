"""Regenerates the Paperless-ngx export fixtures in this directory.

Record shapes follow `document_exporter` in paperless-ngx v2.20.15 and v3.4.2:
a JSON array of Django-serialized records, documents carrying the
`__exported_*_name__` keys, and per-document `*-manifest.json` files with
`--split-manifest`. Secrets below are fake and exist so tests can prove the
reader never yields them.

Run: python3 -I generate.py
"""

import hashlib
import json
import shutil
from pathlib import Path

HERE = Path(__file__).resolve().parent

USER_PASSWORD_HASH = "pbkdf2_sha256$870000$FIXTURESALT$FIXTURE-SECRET-PASSWORD-HASH="
MAIL_PASSWORD = "FIXTURE-SECRET-MAIL-PASSWORD"
SOCIAL_TOKEN = "FIXTURE-SECRET-SOCIAL-TOKEN"
TOTP_SECRET = "FIXTURE-SECRET-TOTP"


def rec(model, pk, fields):
    return {"model": model, "pk": pk, "fields": fields}


def soft_delete(deleted_at=None):
    return {"deleted_at": deleted_at, "restored_at": None, "transaction_id": None}


def matching(name, owner=1):
    return {
        "owner": owner,
        "name": name,
        "match": "",
        "matching_algorithm": 6,
        "is_insensitive": True,
    }


def tag(pk, name, color, parent=None, inbox=False, owner=1):
    return rec(
        "documents.tag",
        pk,
        {
            **matching(name, owner),
            "color": color,
            "is_inbox_tag": inbox,
            "tn_parent": parent,
            "tn_priority": 0,
        },
    )


def taxonomy():
    return [
        rec("documents.correspondent", 1, matching("ACME Corp")),
        rec("documents.correspondent", 2, matching("City Utilities")),
        # Paperless orders tags by name in the export; IDs drive the rename rule
        tag(2, "Finance", "#1f78b4"),
        tag(4, "Home", "#33a02c"),
        tag(1, "Inbox", "#ff7f00", inbox=True),
        tag(3, "Invoices", "#e31a1c", parent=2),
        tag(5, "Invoices", "#fb9a99", parent=4, owner=2),
        rec("documents.documenttype", 1, matching("Invoice")),
        rec("documents.documenttype", 2, matching("Contract")),
        rec(
            "documents.storagepath",
            1,
            {**matching("Archive"), "path": "{{ created_year }}/{{ title }}"},
        ),
        rec(
            "paperless_mail.mailaccount",
            1,
            {
                "owner": 1,
                "name": "Personal mail",
                "imap_server": "imap.example.com",
                "imap_port": 993,
                "imap_security": 2,
                "username": "someone@example.com",
                "password": MAIL_PASSWORD,
                "is_token": False,
                "character_set": "UTF-8",
                "account_type": 1,
                "expiration": None,
            },
        ),
        rec(
            "paperless_mail.mailrule",
            1,
            {"owner": 1, "name": "Invoices", "account": 1, "enabled": True},
        ),
        rec("auth.group", 1, {"name": "family", "permissions": [1, 2]}),
        rec(
            "auth.user",
            1,
            {
                "password": USER_PASSWORD_HASH,
                "last_login": "2025-01-02T08:00:00.123Z",
                "is_superuser": True,
                "username": "admin",
                "first_name": "",
                "last_name": "",
                "email": "admin@example.com",
                "is_staff": True,
                "is_active": True,
                "date_joined": "2023-05-01T10:00:00Z",
                "groups": [],
                "user_permissions": [],
            },
        ),
        rec(
            "auth.user",
            2,
            {
                "password": USER_PASSWORD_HASH,
                "last_login": None,
                "is_superuser": False,
                "username": "partner",
                "first_name": "",
                "last_name": "",
                "email": "",
                "is_staff": False,
                "is_active": True,
                "date_joined": "2023-05-01T10:00:00Z",
                "groups": [1],
                "user_permissions": [],
            },
        ),
        rec("documents.uisettings", 1, {"user": 1, "settings": {"dark_mode": {"enabled": "true"}}}),
        rec("contenttypes.contenttype", 1, {"app_label": "documents", "model": "document"}),
        rec("auth.permission", 1, {"name": "Can view document", "content_type": 1, "codename": "view_document"}),
        rec("auth.permission", 2, {"name": "Can change document", "content_type": 1, "codename": "change_document"}),
        rec("guardian.userobjectpermission", 1, {"permission": 1, "content_type": 1, "object_pk": "1", "user": 2}),
        rec("documents.workflowtrigger", 1, {"type": 1, "sources": [1, 2, 3], "filter_filename": "*.pdf"}),
        rec("documents.workflowaction", 1, {"type": 1, "assign_title": "{correspondent} invoice"}),
        rec("documents.workflow", 1, {"name": "Invoices", "order": 0, "enabled": True, "triggers": [1], "actions": [1]}),
        *custom_fields(),
    ]


def custom_fields():
    def field(pk, name, data_type, extra=None):
        return rec(
            "documents.customfield",
            pk,
            {
                "created": "2024-01-01T09:00:00Z",
                "name": name,
                "data_type": data_type,
                "extra_data": extra or {"select_options": [], "default_currency": None},
            },
        )

    return [
        field(1, "Reference", "string"),
        field(2, "Summary", "longtext"),
        field(3, "Portal", "url"),
        field(4, "Due date", "date"),
        field(5, "Paid", "boolean"),
        field(6, "Pages scanned", "integer"),
        field(7, "Rating", "float"),
        field(8, "Amount", "monetary", {"select_options": [], "default_currency": "EUR"}),
        field(
            9,
            "Priority",
            "select",
            {
                "select_options": [
                    {"id": "hJ3kQ9pLmN2x", "label": "High"},
                    {"id": "aB4cD5eF6gH7", "label": "Low"},
                ],
                "default_currency": None,
            },
        ),
        field(10, "Related", "documentlink"),
    ]


def instance(pk, document, field, deleted_at=None, **values):
    columns = {
        "value_text": None,
        "value_bool": None,
        "value_url": None,
        "value_date": None,
        "value_int": None,
        "value_float": None,
        "value_monetary": None,
        "value_document_ids": None,
        "value_select": None,
        "value_long_text": None,
    }
    for key, value in values.items():
        columns[f"value_{key}"] = value
    return rec(
        "documents.customfieldinstance",
        pk,
        {**soft_delete(deleted_at), "created": "2024-01-16T10:00:00Z", "document": document, "field": field, **columns},
    )


def instances():
    return [
        instance(1, 1, 1, text="INV-42"),
        instance(2, 1, 2, long_text="Quarterly service fee\nincluding support."),
        instance(3, 1, 3, url="https://portal.example.com/invoices/42"),
        instance(4, 1, 4, date="2024-02-15"),
        instance(5, 1, 5, bool=True),
        instance(6, 1, 6, int=3),
        instance(7, 1, 7, float=4.5),
        instance(8, 1, 8, monetary="EUR123.45"),
        instance(9, 1, 9, select="hJ3kQ9pLmN2x"),
        instance(10, 1, 10, document_ids=[2, 99]),
        instance(11, 2, 8, monetary="57.10"),
    ]


def note(pk, document, text, created, deleted_at=None):
    return rec(
        "documents.note",
        pk,
        {**soft_delete(deleted_at), "note": text, "created": created, "document": document, "user": 1},
    )


def document(pk, title, filename, created, *, checksum, correspondent=None, document_type=None,
             tags=(), asn=None, content="", deleted_at=None, mime="application/pdf",
             root=None, version_index=None, version_label=None, added=None, v3=False):
    fields = {
        **soft_delete(deleted_at),
        "owner": 1,
        "correspondent": correspondent,
        "storage_path": None,
        "title": title,
        "document_type": document_type,
        "content": content,
        "mime_type": mime,
        "checksum": checksum,
        "archive_checksum": None,
        "page_count": 1,
        "created": created,
        "modified": added or "2024-03-01T12:30:45.678Z",
        "added": added or f"{created}T09:15:00.250Z",
        "filename": filename,
        "archive_filename": None,
        "original_filename": filename,
        "archive_serial_number": asn,
        "tags": list(tags),
    }
    if v3:
        fields["content_length"] = len(content)
        fields["root_document"] = root
        fields["version_index"] = version_index
        fields["version_label"] = version_label
    else:
        fields["storage_type"] = "unencrypted"
    return rec("documents.document", pk, fields)


def originals(v3):
    docs = [
        dict(pk=1, title="Invoice 42", base="2024-01-15 ACME Corp Invoice 42", created="2024-01-15",
             correspondent=1, document_type=1, tags=[2, 3], asn=1042,
             content="ACME Corp\nInvoice 42\nTotal EUR 123.45", body=b"%PDF-1.4\n% invoice 42\n%%EOF\n"),
        dict(pk=2, title="Water bill", base="2024-02-01 City Utilities Water bill", created="2024-02-01",
             correspondent=2, tags=[1, 4, 5], content="", body=b"%PDF-1.4\n% water bill\n%%EOF\n"),
    ]
    if v3:
        docs += [
            dict(pk=3, title="Old flyer", base="2023-11-30 Old flyer", created="2023-11-30",
                 deleted_at="2025-06-01T08:00:00Z", content="Pizza", body=b"%PDF-1.4\n% flyer\n%%EOF\n"),
            dict(pk=4, title="Lease", base="2024-03-10 Lease", created="2024-03-10", document_type=2,
                 content="Lease agreement", body=b"%PDF-1.4\n% lease draft\n%%EOF\n"),
            # Like upstream's consumer, a version copies only the root's title, created and owner
            dict(pk=5, title="Lease", base="2024-03-10 Lease_01", created="2024-03-10",
                 root=4, version_index=1, version_label="signed", content="Lease agreement signed",
                 added="2024-03-12T10:59:30.125Z", body=b"%PDF-1.4\n% lease signed\n%%EOF\n"),
        ]
    return docs


def notes(v3):
    out = [note(1, 1, "Paid by bank transfer on 2024-02-10.", "2024-02-10T18:22:01.500Z")]
    if v3:
        out.append(note(2, 1, "Wrong amount, ignore.", "2024-02-09T07:00:00Z", deleted_at="2024-02-10T18:00:00Z"))
        out.append(note(3, 4, "Signed copy uploaded as a version.", "2024-03-12T11:00:00Z"))
    return out


def trailing(v3):
    out = [
        rec("socialaccount.socialaccount", 1, {"user": 1, "provider": "oidc", "uid": "abc", "extra_data": {}}),
        rec("socialaccount.socialtoken", 1, {"app": None, "account": 1, "token": SOCIAL_TOKEN, "token_secret": SOCIAL_TOKEN, "expires_at": None}),
        rec("mfa.authenticator", 1, {"user": 1, "type": "totp", "data": {"secret": TOTP_SECRET}}),
    ]
    if v3:
        out.insert(0, rec("documents.sharelink", 1, {**soft_delete(), "created": "2024-01-20T10:00:00Z", "expiration": None, "slug": "Zx9", "document": 1, "owner": 1, "file_version": "original"}))
    return out


def build(name, *, version, v3, split=False, prefix=False, nest=None):
    out = HERE / name
    if out.exists():
        shutil.rmtree(out)
    out.mkdir()
    algo = hashlib.sha256 if v3 else hashlib.md5

    doc_records = []
    for d in originals(v3):
        body = d.pop("body")
        base = d.pop("base")
        pk = d.pop("pk")
        # --use-filename-format can put originals and split manifests in any directory
        if nest and pk in nest:
            base = f"{nest[pk]}/{base}"
        rel = f"{base}.pdf"
        rel_original = f"originals/{rel}" if prefix else rel
        target = out / rel_original
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(body)
        r = document(pk, d.pop("title"), f"{pk:07}.pdf", d.pop("created"), checksum=algo(body).hexdigest(), v3=v3, **d)
        r["__exported_file_name__"] = rel_original
        r["__exported_thumbnail_name__"] = f"thumbnails/{base}-thumbnail.webp" if prefix else f"{base}-thumbnail.webp"
        doc_records.append((base, r))

    # A v2-consumed document in a v3 export keeps its MD5 checksum
    if v3:
        base, lease = doc_records[3]
        lease["fields"]["checksum"] = hashlib.md5((out / lease["__exported_file_name__"]).read_bytes()).hexdigest()

    the_notes = notes(v3)
    the_instances = instances()
    head = taxonomy()
    if not split:
        head += the_instances
    head.append(rec("paperless.applicationconfiguration", 1, {"output_type": None, "pages": None}))
    if not split:
        head += the_notes
        head += [r for _, r in doc_records]
    else:
        for base, r in doc_records:
            pk = r["pk"]
            content = [r]
            content += [n for n in the_notes if n["fields"]["document"] == pk]
            content += [i for i in the_instances if i["fields"]["document"] == pk]
            manifest = out / ("json/" if prefix else "") / f"{base}-manifest.json"
            manifest.parent.mkdir(parents=True, exist_ok=True)
            manifest.write_text(json.dumps(content, indent=2, ensure_ascii=False))
    head += trailing(v3)
    (out / "manifest.json").write_text(json.dumps(head, indent=2, ensure_ascii=False))
    (out / "metadata.json").write_text(json.dumps({"version": version}, indent=2))


build("v2_20", version="2.20.15", v3=False)
build("v3_4", version="3.4.2", v3=True)
build("v3_4_split", version="3.4.2", v3=True, split=True)
build("v3_4_folder_prefix", version="3.4.2", v3=True, prefix=True, split=True)
build("v3_4_split_archive", version="3.4.2", v3=True, split=True, nest={4: "archive/2024", 5: "archive/2024"})
