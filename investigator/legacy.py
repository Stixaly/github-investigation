import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path
from uuid import uuid4

import ijson
from ijson.common import ObjectBuilder
from psycopg.types.json import Jsonb

from .db import connect
from .security import require_access
from .store import account, entity, evidence, relation


def file_hash(path):
    result = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def repo_items(path):
    """Yield individual repositories without loading a user's complete repository list."""
    owner, builder, depth = None, None, 0
    with path.open("rb") as handle:
        for prefix, event, value in ijson.parse(handle, use_float=True):
            if prefix == "" and event == "map_key":
                owner = value
            if builder is None and prefix == f"{owner}.item" and event == "start_map":
                builder, depth = ObjectBuilder(), 0
            if builder is not None:
                builder.event(event, value)
                if event in ("start_map", "start_array"):
                    depth += 1
                elif event in ("end_map", "end_array"):
                    depth -= 1
                if depth == 0:
                    yield owner, builder.value
                    builder = None


def import_snapshot(directory, investigation_id, login, observed_date=None):
    directory = Path(directory).resolve()
    users_path, repos_path = directory / "users_db.json", directory / "repos_db.json"
    # Only these inert files are read. Never import, inspect, or load sets.pkl/token_key.txt.
    digests = {name: file_hash(path) for name, path in (("users", users_path), ("repos", repos_path))}
    manifest = {"files": digests, "observed_date": observed_date, "version": 1}
    key = hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest()
    observed = datetime.strptime(observed_date, "%Y-%m-%d").replace(tzinfo=timezone.utc) if observed_date else None
    precision = "day" if observed else "unknown"
    users, counters = {}, {"users": 0, "repositories": 0, "user_errors": 0, "skipped": 0}
    with connect() as conn:
        user = conn.execute("SELECT id FROM users WHERE login=%s", (login.lower(),)).fetchone()
        if not user:
            raise ValueError("Compte analyste introuvable")
        require_access(conn, investigation_id, user["id"], write=True)
        existing = conn.execute("SELECT * FROM scans WHERE investigation_id=%s AND import_key=%s",
                                (investigation_id, key)).fetchone()
        if existing and existing["status"] == "succeeded":
            return {"scan_id": str(existing["id"]), "reused": True}
        if existing:
            scan_id = existing["id"]
        else:
            scan_id = uuid4()
            conn.execute(
                "INSERT INTO scans(id,investigation_id,requested_by,source,seed,status,import_key,idempotency_key,started_at) "
                "VALUES(%s,%s,%s,'legacy',%s,'running',%s,%s,now())",
                (scan_id, investigation_id, user["id"], directory.name, key, uuid4())
            )
    try:
        with users_path.open("rb") as handle, connect() as conn:
            for name, data in ijson.kvitems(handle, "", use_float=True):
                if not isinstance(data, dict) or "error" in data:
                    counters["user_errors"] += 1
                    continue
                proof = evidence(conn, data, f"legacy:sha256:{digests['users']}:users_db.json:{name}")
                identity = account(conn, scan_id, data, proof, observed, precision)
                if identity:
                    users[name.lower()] = identity
                    counters["users"] += 1
                else:
                    counters["skipped"] += 1
                if counters["users"] % 200 == 0:
                    conn.commit()
        with connect() as conn:
            for owner_name, data in repo_items(repos_path):
                if not data.get("full_name") or data.get("private"):
                    counters["skipped"] += 1
                    continue
                proof = evidence(conn, data, f"legacy:sha256:{digests['repos']}:repos_db.json:{owner_name}")
                stable = data.get("id")
                provisional = stable is None
                external = str(stable) if stable is not None else f"legacy:{digests['repos']}:{data['full_name'].lower()}"
                target = entity(conn, scan_id, "Repository", external, data["full_name"], data,
                                proof, observed, precision, provisional)
                owner = users.get(owner_name.lower())
                # Confirm the name agrees with the owner bucket; preserve uncertainty otherwise.
                if owner and data["full_name"].split("/")[0].lower() == owner_name.lower():
                    relation(conn, scan_id, owner, "OWNS", target, proof, observed, precision=precision)
                counters["repositories"] += 1
                if counters["repositories"] % 200 == 0:
                    conn.commit()
            coverage = {"import": {"status": "complete", **counters, "manifest": manifest},
                        "roles": {"status": "not_imported", "reason": "Pickle non chargé ; relations historiques non reconstruites"}}
            conn.execute("UPDATE scans SET status='succeeded',finished_at=now(),coverage=%s,error=NULL WHERE id=%s",
                         (Jsonb(coverage), scan_id))
    except Exception:
        with connect() as conn:
            conn.execute("UPDATE scans SET status='failed',error='Import interrompu ; relancer le même import' WHERE id=%s", (scan_id,))
        raise
    return {"scan_id": str(scan_id), **counters, "reused": False}
