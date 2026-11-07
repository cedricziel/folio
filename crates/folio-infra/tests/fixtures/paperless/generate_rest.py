"""Regenerates the Paperless-ngx REST recordings in this directory.

Each recording describes the library of an export fixture (`v2_20`, `v3_4`)
as the REST API serves it, so both adapters can be checked against the same
library. Run `generate.py` first; this script reads its manifests.

Response shapes follow `src/documents/serialisers.py` and `views.py` of
paperless-ngx v2.20.15 and v3.4.2 with `Accept: application/json; version=9`:

- `StandardPagination`: `count`, `next`, `previous`, `all`, `results`
- datetimes in the server's time zone (here +01:00), `created` as a date
- `fields=` drops everything not asked for (`DynamicFieldsModelSerializer`)
- notes are the live ones only, with the nested author
- v3 lists only root documents; each carries `versions`, its `content` is
  the newest version's, and `/metadata/` and `/download/` resolve to the
  newest version unless `?version=` names one
- `/api/trash/` uses the full `DocumentSerializer`; `/metadata/` 404s for
  trashed documents

Run: python3 -I generate_rest.py
"""

import json
from datetime import datetime, timedelta, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
TZ = timezone(timedelta(hours=1))
BASE = "http://paperless.test"
PAGE_SIZE = 100
ID_PAGE_SIZE = 1000
DOC_FIELDS = (
    "id,correspondent,document_type,title,content,tags,created,modified,added,"
    "deleted_at,archive_serial_number,original_file_name,mime_type,notes,"
    "custom_fields,root_document,versions"
)
USERS = {1: "admin", 2: "partner"}


def local(ts):
    """DRF renders datetimes in the current time zone"""
    if ts is None:
        return None
    parsed = datetime.fromisoformat(ts.replace("Z", "+00:00"))
    return parsed.astimezone(TZ).isoformat()


def page(results, all_ids=None):
    return {
        "count": len(results),
        "next": None,
        "previous": None,
        "all": all_ids if all_ids is not None else [r["id"] for r in results],
        "results": results,
    }


def entry(path, query, body, status=200, body_file=None, headers=None):
    out = {"method": "GET", "path": path, "query": query, "status": status}
    if headers:
        out["headers"] = headers
    if body_file is not None:
        out["body_file"] = body_file
    else:
        out["body"] = body
    return out


def matching(pk, f, count):
    return {
        "id": pk,
        "slug": f["name"].lower().replace(" ", "-"),
        "name": f["name"],
        "match": f["match"],
        "matching_algorithm": f["matching_algorithm"],
        "is_insensitive": f["is_insensitive"],
        "document_count": count,
        "owner": f["owner"],
        "user_can_change": True,
    }


def build(name, *, version, v3):
    export = HERE / name
    manifest = json.loads((export / "manifest.json").read_text())
    by_model = {}
    for r in manifest:
        by_model.setdefault(r["model"], []).append(r)

    docs = sorted(by_model["documents.document"], key=lambda r: r["pk"])
    notes = by_model.get("documents.note", [])
    instances = by_model.get("documents.customfieldinstance", [])
    fields = {r["pk"]: r["fields"] for r in by_model["documents.customfield"]}
    live = [d for d in docs if d["fields"]["deleted_at"] is None]
    roots = [d for d in live if not d["fields"].get("root_document")]
    trashed = [d for d in docs if d["fields"]["deleted_at"] is not None and not d["fields"].get("root_document")]

    def count_docs(pred):
        return sum(1 for d in roots if pred(d["fields"]))

    def tag_json(r):
        f = r["fields"]
        children = [t for t in by_model["documents.tag"] if t["fields"]["tn_parent"] == r["pk"]]
        body = {
            **matching(r["pk"], f, count_docs(lambda d: r["pk"] in d["tags"])),
            "color": f["color"],
            "text_color": "#ffffff",
            "is_inbox_tag": f["is_inbox_tag"],
            "parent": f["tn_parent"],
            # v2 nests child tags, v3 lists their IDs
            "children": [c["pk"] for c in children] if v3 else [tag_json(c) for c in children],
        }
        return body

    def versions_of(root):
        if not v3:
            return None
        family = [d for d in docs if d["fields"].get("root_document") == root["pk"]] + [root]
        family.sort(key=lambda d: (d["fields"].get("version_index") or 0, d["pk"]), reverse=True)
        return [
            {
                "id": d["pk"],
                "added": local(d["fields"]["added"]),
                "version_label": d["fields"].get("version_label"),
                "checksum": d["fields"]["checksum"],
                "is_root": d["pk"] == root["pk"],
            }
            for d in family
        ]

    def newest(root):
        family = versions_of(root)
        return next(d for d in docs if d["pk"] == family[0]["id"]) if family else root

    def value_of(i):
        f = i["fields"]
        data_type = fields[f["field"]]["data_type"]
        column = {
            "string": "value_text",
            "longtext": "value_long_text",
            "url": "value_url",
            "date": "value_date",
            "boolean": "value_bool",
            "integer": "value_int",
            "float": "value_float",
            "monetary": "value_monetary",
            "select": "value_select",
            "documentlink": "value_document_ids",
        }[data_type]
        return f[column]

    def document_json(r, *, effective_content):
        f = r["fields"]
        content = newest(r)["fields"]["content"] if effective_content else f["content"]
        body = {
            "id": r["pk"],
            "correspondent": f["correspondent"],
            "document_type": f["document_type"],
            "storage_path": f["storage_path"],
            "title": f["title"],
            "content": content,
            "tags": f["tags"],
            "created": f["created"],
            "created_date": f["created"],
            "modified": local(f["modified"]),
            "added": local(f["added"]),
            "deleted_at": local(f["deleted_at"]),
            "archive_serial_number": f["archive_serial_number"],
            "original_file_name": f["original_filename"],
            "archived_file_name": None,
            "owner": f["owner"],
            "user_can_change": True,
            "is_shared_by_requester": False,
            "notes": [
                {
                    "id": n["pk"],
                    "note": n["fields"]["note"],
                    "created": local(n["fields"]["created"]),
                    "user": {"id": n["fields"]["user"], "username": USERS[n["fields"]["user"]], "first_name": "", "last_name": ""},
                }
                for n in notes
                if n["fields"]["document"] == r["pk"] and n["fields"]["deleted_at"] is None
            ],
            "custom_fields": [
                {"value": value_of(i), "field": i["fields"]["field"]}
                for i in instances
                if i["fields"]["document"] == r["pk"] and i["fields"]["deleted_at"] is None
            ],
            "page_count": f["page_count"],
            "mime_type": f["mime_type"],
        }
        if v3:
            body["duplicate_documents"] = []
            body["root_document"] = f.get("root_document")
            body["versions"] = versions_of(r)
        return body

    def only(body, wanted):
        keep = wanted.split(",")
        return {k: v for k, v in body.items() if k in keep}

    def metadata(d):
        f = d["fields"]
        original = export / d["__exported_file_name__"]
        body = {
            "original_checksum": f["checksum"],
            "original_size": original.stat().st_size,
            "original_mime_type": f["mime_type"],
            "media_filename": f["filename"],
            "has_archive_version": False,
            "original_metadata": [],
            "archive_checksum": None,
            "archive_media_filename": None,
            "original_filename": f["original_filename"],
            "archive_size": None,
            "archive_metadata": None,
        }
        if v3:
            body["barcodes"] = []
        else:
            body["lang"] = "en"
        return body

    recording = [
        # `/api/` redirects to the HTML schema viewer, so the version comes from here
        entry("/api/documents/", {"page": "1", "page_size": "1"}, {**page([{"id": roots[0]["pk"]}]), "count": len(roots), "next": f"{BASE}/api/documents/?fields=id&page=2&page_size=1", "all": [r["pk"] for r in roots]}, headers={"X-Version": version, "X-Api-Version": "10" if v3 else "9"}),
        entry("/api/correspondents/", {"page": "1", "page_size": str(PAGE_SIZE), "ordering": "id"}, page([
            {**matching(r["pk"], r["fields"], count_docs(lambda d: d["correspondent"] == r["pk"])), "last_correspondence": None}
            for r in sorted(by_model["documents.correspondent"], key=lambda r: r["pk"])
        ])),
        entry("/api/document_types/", {"page": "1", "page_size": str(PAGE_SIZE), "ordering": "id"}, page([
            matching(r["pk"], r["fields"], count_docs(lambda d: d["document_type"] == r["pk"]))
            for r in sorted(by_model["documents.documenttype"], key=lambda r: r["pk"])
        ])),
        entry("/api/tags/", {"page": "1", "page_size": str(PAGE_SIZE), "ordering": "id"}, page([
            tag_json(r) for r in sorted(by_model["documents.tag"], key=lambda r: r["pk"])
        ])),
        entry("/api/custom_fields/", {"page": "1", "page_size": str(PAGE_SIZE), "ordering": "id"}, page([
            {
                "id": pk,
                "name": f["name"],
                "data_type": f["data_type"],
                "extra_data": f["extra_data"],
                "document_count": sum(1 for i in instances if i["fields"]["field"] == pk),
            }
            for pk, f in sorted(fields.items())
        ])),
        # Every document shares one `modified`, so a single keyset page holds them all
        entry("/api/documents/", {"page": "1", "page_size": str(PAGE_SIZE), "ordering": "modified,id"}, page([
            only(document_json(r, effective_content=True), DOC_FIELDS) for r in roots
        ])),
        entry("/api/documents/", {"page": "1", "page_size": str(ID_PAGE_SIZE), "ordering": "id"}, page([
            {"id": r["pk"]} for r in roots
        ])),
        entry("/api/trash/", {"page": "1", "page_size": str(PAGE_SIZE)}, page([
            document_json(r, effective_content=False) for r in trashed
        ])),
    ]

    for d in docs:
        f = d["fields"]
        root = f.get("root_document")
        if f["deleted_at"] is not None:
            # Trashed documents: /metadata/ 404s, /download/ still serves them
            recording.append(entry(f"/api/documents/{d['pk']}/metadata/", {}, {"detail": "No Document matches the given query."}, status=404))
            recording.append(entry(f"/api/documents/{d['pk']}/download/", {"original": "true"}, None, body_file=f"{name}/{d['__exported_file_name__']}"))
            continue
        has_versions = v3 and root is None and len(versions_of(d)) > 1
        if root is not None:
            # A version: /metadata/ and /download/ on its own ID serve it
            recording.append(entry(f"/api/documents/{d['pk']}/metadata/", {}, metadata(d)))
            recording.append(entry(f"/api/documents/{d['pk']}/download/", {"original": "true"}, None, body_file=f"{name}/{d['__exported_file_name__']}"))
            recording.append(entry(f"/api/documents/{root}/", {"version": str(d["pk"]), "fields": "content"}, {"content": f["content"]}))
        elif has_versions:
            # A root with versions: without ?version= the newest version answers
            version = {"version": str(d["pk"])}
            recording.append(entry(f"/api/documents/{d['pk']}/metadata/", version, metadata(d)))
            recording.append(entry(f"/api/documents/{d['pk']}/download/", {"original": "true", **version}, None, body_file=f"{name}/{d['__exported_file_name__']}"))
            recording.append(entry(f"/api/documents/{d['pk']}/", {**version, "fields": "content"}, {"content": f["content"]}))
        else:
            recording.append(entry(f"/api/documents/{d['pk']}/metadata/", {}, metadata(d)))
            recording.append(entry(f"/api/documents/{d['pk']}/download/", {"original": "true"}, None, body_file=f"{name}/{d['__exported_file_name__']}"))

    out = HERE / f"rest_{name}.json"
    out.write_text(json.dumps(recording, indent=2, ensure_ascii=False) + "\n")


build("v2_20", version="2.20.15", v3=False)
build("v3_4", version="3.4.2", v3=True)
