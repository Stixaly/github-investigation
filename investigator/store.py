import hashlib
import json
from uuid import uuid4

from psycopg.types.json import Jsonb


def evidence(conn, payload, source):
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    digest = hashlib.sha256((source + "\n" + canonical).encode()).hexdigest()
    conn.execute(
        "INSERT INTO evidence(hash,payload,source) VALUES(%s,%s,%s) ON CONFLICT DO NOTHING",
        (digest, Jsonb(payload), source),
    )
    return digest


def entity(conn, scan_id, kind, external_id, label, data, proof, observed, precision="instant", provisional=False):
    row = conn.execute(
        "INSERT INTO entities(id,kind,external_id,label,provisional) VALUES(%s,%s,%s,%s,%s) "
        "ON CONFLICT(kind,external_id) DO UPDATE SET label=excluded.label RETURNING id",
        (uuid4(), kind, str(external_id), label, provisional),
    ).fetchone()
    conn.execute(
        "INSERT INTO scan_entities(scan_id,entity_id,evidence_hash,data,observed_at,time_precision) "
        "VALUES(%s,%s,%s,%s,%s,%s) ON CONFLICT DO NOTHING",
        (scan_id, row["id"], proof, Jsonb(data), observed, precision),
    )
    return row["id"]


def relation(conn, scan_id, source, kind, target, proof, observed, context="", event_at=None, precision="instant"):
    row = conn.execute(
        "INSERT INTO relations(id,source_id,kind,target_id,context) VALUES(%s,%s,%s,%s,%s) "
        "ON CONFLICT(source_id,kind,target_id,context) DO UPDATE SET context=excluded.context RETURNING id",
        (uuid4(), source, kind, target, context),
    ).fetchone()
    conn.execute(
        "INSERT INTO scan_relations(scan_id,relation_id,evidence_hash,observed_at,source_event_at,time_precision) "
        "VALUES(%s,%s,%s,%s,%s,%s) ON CONFLICT DO NOTHING",
        (scan_id, row["id"], proof, observed, event_at, precision),
    )


def account(conn, scan_id, data, proof, observed, precision="instant"):
    if not data or data.get("id") is None or not data.get("login"):
        return None
    return entity(conn, scan_id, "Account", data["id"], data["login"], data, proof, observed, precision)


def repository(conn, scan_id, data, proof, observed):
    if data.get("private") or data.get("visibility", "public") != "public":
        return None
    if data.get("id") is None or not data.get("full_name"):
        return None
    target = entity(conn, scan_id, "Repository", data["id"], data["full_name"], data, proof, observed)
    owner = account(conn, scan_id, data.get("owner"), proof, observed)
    if owner:
        relation(conn, scan_id, owner, "OWNS", target, proof, observed)
    return target


def ingest_github(conn, scan_id, role, item, source_url, observed, root_id=None):
    if role == "events" and item.get("public") is not True:
        return None
    if role in ("repository", "forks", "repositories") and (
        item.get("private") or item.get("visibility", "public") != "public"
    ):
        return None
    proof = evidence(conn, item, source_url)
    if role in ("repository", "forks", "repositories"):
        target = repository(conn, scan_id, item, proof, observed)
        if target and role == "forks" and root_id:
            relation(conn, scan_id, target, "FORK_OF", root_id, proof, observed)
        if target and item.get("parent"):
            parent = repository(conn, scan_id, item["parent"], proof, observed)
            if parent:
                relation(conn, scan_id, target, "FORK_OF", parent, proof, observed)
        return target
    if role in ("account", "contributors"):
        actor = account(conn, scan_id, item, proof, observed)
        if actor and role == "contributors" and root_id:
            relation(conn, scan_id, actor, "CONTRIBUTED_TO", root_id, proof, observed)
        return actor
    if role == "issues" and root_id and item.get("id"):
        kind = "PullRequest" if item.get("pull_request") else "Issue"
        target = entity(conn, scan_id, kind, item["id"], f"#{item.get('number')} {item.get('title', '')}", item, proof, observed)
        actor = account(conn, scan_id, item.get("user"), proof, observed)
        if actor:
            relation(conn, scan_id, actor, "AUTHORED", target, proof, observed, event_at=item.get("created_at"))
        relation(conn, scan_id, target, "IN_REPOSITORY", root_id, proof, observed)
        return target
    if role == "events" and root_id and item.get("public") is True:
        actor = account(conn, scan_id, item.get("actor"), proof, observed)
        if actor and item.get("type") == "WatchEvent":
            relation(conn, scan_id, actor, "STARRED", root_id, proof, observed, event_at=item.get("created_at"))
        if actor and item.get("type") == "ForkEvent":
            fork = repository(conn, scan_id, item.get("payload", {}).get("forkee", {}), proof, observed)
            if fork:
                relation(conn, scan_id, fork, "FORK_OF", root_id, proof, observed)
        return actor
    return None
