from contextlib import asynccontextmanager
from pathlib import Path
from typing import Literal
from urllib.parse import urlsplit
from uuid import UUID, uuid4

from fastapi import Depends, FastAPI, HTTPException, Query, Request, Response
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel, Field
from starlette.middleware.trustedhost import TrustedHostMiddleware

from .db import connect
from .domain import parse_seed
from .security import (
    COOKIE,
    ORIGIN,
    SECURE,
    accessible_scan,
    authenticate,
    check_origin,
    csrf,
    current_user,
    require_access,
    token_hash,
    validate_origin,
)


@asynccontextmanager
async def lifespan(app):
    validate_origin()
    with connect() as conn:
        rows = conn.execute("SELECT version FROM schema_migrations").fetchall()
        if rows != [{"version": 1}]:
            raise RuntimeError("Schema incompatible; run migrations first")
    yield


app = FastAPI(title="GitHub Investigation", version="0.1.0", lifespan=lifespan)
app.add_middleware(TrustedHostMiddleware, allowed_hosts=[urlsplit(ORIGIN).hostname, "localhost", "127.0.0.1", "api"])


@app.middleware("http")
async def browser_headers(request, call_next):
    response = await call_next(request)
    response.headers["X-Content-Type-Options"] = "nosniff"
    response.headers["Referrer-Policy"] = "no-referrer"
    response.headers["Content-Security-Policy"] = (
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; "
        "img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"
    )
    response.headers["Cache-Control"] = "no-store"
    if SECURE:
        response.headers["Strict-Transport-Security"] = "max-age=31536000"
    return response


class Login(BaseModel):
    login: str = Field(min_length=3, max_length=40)
    password: str = Field(min_length=1, max_length=1024)


class InvestigationInput(BaseModel):
    title: str = Field(min_length=1, max_length=160)
    description: str = Field(default="", max_length=5000)


class ScanInput(BaseModel):
    seed: str = Field(min_length=1, max_length=250)
    max_requests: int = Field(default=12, ge=1, le=50)
    idempotency_key: UUID = Field(default_factory=uuid4)


class MemberInput(BaseModel):
    login: str = Field(min_length=3, max_length=40)
    role: Literal["analyst", "reader"]


class NoteInput(BaseModel):
    body: str = Field(min_length=1, max_length=10000)


@app.get("/health")
def health():
    return {"status": "ok"}


@app.get("/ready")
def ready():
    with connect() as conn:
        conn.execute("SELECT 1")
    return {"status": "ready"}


@app.post("/api/v1/login")
def login(data: Login, request: Request, response: Response):
    check_origin(request)
    if request.headers.get("content-type", "").split(";")[0] != "application/json":
        raise HTTPException(415, "JSON requis")
    user, token = authenticate(data.login, data.password)
    response.set_cookie(COOKIE, token, httponly=True, secure=SECURE, samesite="strict", max_age=28800, path="/")
    return {"id": user["id"], "login": user["login"], "csrf": csrf(token)}


@app.get("/api/v1/me")
def me(request: Request, user=Depends(current_user)):
    return {**user, "csrf": csrf(request.cookies[COOKIE])}


@app.post("/api/v1/logout", status_code=204)
def logout(request: Request, response: Response, user=Depends(current_user)):
    with connect() as conn:
        conn.execute("DELETE FROM sessions WHERE token_hash=%s", (token_hash(request.cookies[COOKIE]),))
    response.delete_cookie(COOKIE, path="/", secure=SECURE, httponly=True, samesite="strict")


@app.get("/api/v1/investigations")
def investigations(user=Depends(current_user)):
    with connect() as conn:
        return conn.execute(
            "SELECT i.*,m.role,(SELECT count(*) FROM scans s WHERE s.investigation_id=i.id) AS scan_count "
            "FROM investigations i JOIN memberships m ON m.investigation_id=i.id "
            "WHERE m.user_id=%s ORDER BY i.created_at DESC LIMIT 200", (user["id"],)
        ).fetchall()


@app.post("/api/v1/investigations", status_code=201)
def create_investigation(data: InvestigationInput, user=Depends(current_user)):
    with connect() as conn:
        row = conn.execute("INSERT INTO investigations(id,title,description) VALUES(%s,%s,%s) RETURNING *",
                           (uuid4(), data.title, data.description)).fetchone()
        conn.execute("INSERT INTO memberships VALUES(%s,%s,'owner')", (row["id"], user["id"]))
        conn.execute("INSERT INTO audit_events(actor_id,action,investigation_id) VALUES(%s,'investigation.create',%s)",
                     (user["id"], row["id"]))
        return row


@app.get("/api/v1/investigations/{investigation_id}/members")
def members(investigation_id: UUID, user=Depends(current_user)):
    with connect() as conn:
        require_access(conn, investigation_id, user["id"])
        return conn.execute("SELECT u.login,m.role FROM memberships m JOIN users u ON u.id=m.user_id "
                            "WHERE m.investigation_id=%s ORDER BY u.login", (investigation_id,)).fetchall()


@app.put("/api/v1/investigations/{investigation_id}/members")
def add_member(investigation_id: UUID, data: MemberInput, user=Depends(current_user)):
    with connect() as conn:
        require_access(conn, investigation_id, user["id"], owner=True)
        target = conn.execute("SELECT id FROM users WHERE login=%s", (data.login.lower(),)).fetchone()
        if not target:
            raise HTTPException(404, "Compte local introuvable")
        if target["id"] == user["id"]:
            raise HTTPException(409, "Le propriétaire conserve son rôle")
        conn.execute("INSERT INTO memberships VALUES(%s,%s,%s) ON CONFLICT(investigation_id,user_id) "
                     "DO UPDATE SET role=excluded.role", (investigation_id, target["id"], data.role))
        conn.execute("INSERT INTO audit_events(actor_id,action,investigation_id) VALUES(%s,'membership.update',%s)",
                     (user["id"], investigation_id))
    return {"status": "ok"}


@app.delete("/api/v1/investigations/{investigation_id}/members/{login_name}", status_code=204)
def remove_member(investigation_id: UUID, login_name: str, user=Depends(current_user)):
    with connect() as conn:
        require_access(conn, investigation_id, user["id"], owner=True)
        conn.execute("DELETE FROM memberships WHERE investigation_id=%s AND role!='owner' "
                     "AND user_id IN (SELECT id FROM users WHERE login=%s)", (investigation_id, login_name.lower()))
        conn.execute("INSERT INTO audit_events(actor_id,action,investigation_id) VALUES(%s,'membership.remove',%s)",
                     (user["id"], investigation_id))


@app.get("/api/v1/investigations/{investigation_id}/scans")
def list_scans(investigation_id: UUID, user=Depends(current_user)):
    with connect() as conn:
        require_access(conn, investigation_id, user["id"])
        return conn.execute("SELECT * FROM scans WHERE investigation_id=%s ORDER BY requested_at DESC LIMIT 100",
                            (investigation_id,)).fetchall()


@app.post("/api/v1/investigations/{investigation_id}/scans", status_code=202)
def create_scan(investigation_id: UUID, data: ScanInput, user=Depends(current_user)):
    try:
        _, seed = parse_seed(data.seed)
    except ValueError as exc:
        raise HTTPException(422, str(exc)) from exc
    with connect() as conn:
        require_access(conn, investigation_id, user["id"], write=True)
        # Serialize per investigation to enforce the cap under concurrent submissions.
        conn.execute("SELECT id FROM investigations WHERE id=%s FOR UPDATE", (investigation_id,))
        existing = conn.execute("SELECT * FROM scans WHERE investigation_id=%s AND idempotency_key=%s",
                                (investigation_id, data.idempotency_key)).fetchone()
        if existing:
            if existing["seed"] != seed or existing["max_requests"] != data.max_requests:
                raise HTTPException(409, "Clé d'idempotence déjà utilisée pour un autre scan")
            return existing
        pending = conn.execute("SELECT count(*) AS n FROM scans WHERE investigation_id=%s "
                               "AND status IN ('queued','running','waiting_quota')", (investigation_id,)).fetchone()["n"]
        if pending >= 3:
            raise HTTPException(429, "Trois scans sont déjà en attente ou actifs")
        return conn.execute(
            "INSERT INTO scans(id,investigation_id,requested_by,source,seed,status,max_requests,idempotency_key) "
            "VALUES(%s,%s,%s,'github',%s,'queued',%s,%s) RETURNING *",
            (uuid4(), investigation_id, user["id"], seed, data.max_requests, data.idempotency_key)
        ).fetchone()


@app.post("/api/v1/scans/{scan_id}/cancel")
def cancel(scan_id: UUID, user=Depends(current_user)):
    with connect() as conn:
        accessible_scan(conn, scan_id, user["id"], write=True)
        conn.execute("UPDATE scans SET status='cancelled',generation=generation+1,finished_at=now() "
                     "WHERE id=%s AND status IN ('queued','running','waiting_quota')", (scan_id,))
    return {"status": "ok"}


@app.get("/api/v1/scans/{scan_id}/graph")
def graph(scan_id: UUID, limit: int = Query(300, ge=1, le=500), q: str = Query("", max_length=120),
          focus: UUID | None = None, user=Depends(current_user)):
    with connect() as conn:
        scan = accessible_scan(conn, scan_id, user["id"])
        # Prioritize the seed and accounts, then render the induced bounded subgraph.
        ids = conn.execute(
            "SELECT e.id FROM entities e JOIN scan_entities se ON se.entity_id=e.id "
            "WHERE se.scan_id=%s AND (%s='' OR coalesce(se.data->>'full_name',se.data->>'login',"
            "se.data->>'title',e.external_id) ILIKE %s) "
            "AND (%s::uuid IS NULL OR e.id=%s OR e.id IN ("
            "SELECT CASE WHEN r.source_id=%s THEN r.target_id ELSE r.source_id END "
            "FROM relations r JOIN scan_relations sr ON sr.relation_id=r.id WHERE sr.scan_id=%s "
            "AND (r.source_id=%s OR r.target_id=%s) LIMIT 1000)) "
            "GROUP BY e.id ORDER BY bool_or(coalesce(se.data->>'full_name',se.data->>'login',"
            "se.data->>'title',e.external_id)=%s) DESC,(e.kind='Account') DESC,e.id LIMIT %s",
            (scan_id, q, '%' + q + '%', focus, focus, focus, scan_id, focus, focus, scan["seed"], limit + 1)
        ).fetchall()
        selected = [r["id"] for r in ids[:limit]]
        nodes = conn.execute(
            "SELECT DISTINCT ON (e.id) e.id,e.kind,e.external_id,e.provisional,se.data,se.observed_at,"
            "se.time_precision,se.evidence_hash,se.recorded_at FROM entities e JOIN scan_entities se ON se.entity_id=e.id "
            "WHERE se.scan_id=%s AND e.id=ANY(%s::uuid[]) ORDER BY e.id,se.recorded_at DESC,se.evidence_hash",
            (scan_id, selected)
        ).fetchall()
        edges = conn.execute(
            "SELECT DISTINCT ON (r.id) r.*,sr.evidence_hash,sr.observed_at,sr.source_event_at,sr.time_precision "
            "FROM relations r JOIN scan_relations sr ON sr.relation_id=r.id "
            "WHERE sr.scan_id=%s AND r.source_id=ANY(%s::uuid[]) AND r.target_id=ANY(%s::uuid[]) "
            "ORDER BY r.id,sr.recorded_at DESC LIMIT 2001", (scan_id, selected, selected)
        ).fetchall()
        for node in nodes:
            data = node["data"]
            node["label"] = data.get("full_name") or data.get("login") or data.get("title") or str(node["external_id"])
        return {"nodes": nodes, "edges": edges[:2000], "coverage": scan["coverage"], "scan_id": scan_id,
                "temporal_mode": "scan", "truncated": len(ids) > limit or len(edges) > 2000,
                "notice": "Vue des observations de ce scan. Une absence ne signifie pas une suppression."}


@app.get("/api/v1/scans/{scan_id}/compare/{other_id}")
def compare(scan_id: UUID, other_id: UUID, user=Depends(current_user)):
    with connect() as conn:
        first = accessible_scan(conn, scan_id, user["id"])
        second = accessible_scan(conn, other_id, user["id"])
        counts = conn.execute(
            "WITH a AS (SELECT DISTINCT relation_id FROM scan_relations WHERE scan_id=%s), "
            "b AS (SELECT DISTINCT relation_id FROM scan_relations WHERE scan_id=%s) "
            "SELECT count(*) FILTER(WHERE a.relation_id IS NULL) AS newly_observed, "
            "count(*) FILTER(WHERE b.relation_id IS NULL) AS not_reobserved, "
            "count(*) FILTER(WHERE a.relation_id IS NOT NULL AND b.relation_id IS NOT NULL) AS shared "
            "FROM a FULL JOIN b USING(relation_id)", (scan_id, other_id)
        ).fetchone()
        return {**counts, "from": first["id"], "to": second["id"],
                "coverage_from": first["coverage"], "coverage_to": second["coverage"],
                "notice": "Non réobservé ne signifie pas supprimé. Comparer les couvertures."}


@app.get("/api/v1/entities/{entity_id}/overlaps")
def overlaps(entity_id: UUID, user=Depends(current_user)):
    with connect() as conn:
        return conn.execute(
            "SELECT DISTINCT i.id,i.title FROM investigations i JOIN memberships m ON m.investigation_id=i.id "
            "JOIN scans s ON s.investigation_id=i.id JOIN scan_entities se ON se.scan_id=s.id "
            "WHERE m.user_id=%s AND se.entity_id=%s ORDER BY i.title LIMIT 100", (user["id"], entity_id)
        ).fetchall()


@app.get("/api/v1/evidence/{digest}")
def get_evidence(digest: str, user=Depends(current_user)):
    with connect() as conn:
        row = conn.execute(
            "SELECT e.* FROM evidence e WHERE e.hash=%s AND EXISTS("
            "SELECT 1 FROM scan_entities se JOIN scans s ON s.id=se.scan_id "
            "JOIN memberships m ON m.investigation_id=s.investigation_id "
            "WHERE se.evidence_hash=e.hash AND m.user_id=%s)", (digest, user["id"])
        ).fetchone()
        if not row:
            raise HTTPException(404, "Preuve introuvable")
        return row


@app.get("/api/v1/investigations/{investigation_id}/notes")
def notes(investigation_id: UUID, user=Depends(current_user)):
    with connect() as conn:
        require_access(conn, investigation_id, user["id"])
        return conn.execute("SELECT a.id,a.body,a.created_at,u.login FROM annotations a JOIN users u ON u.id=a.author_id "
                            "WHERE investigation_id=%s ORDER BY a.created_at DESC LIMIT 100", (investigation_id,)).fetchall()


@app.post("/api/v1/investigations/{investigation_id}/notes", status_code=201)
def add_note(investigation_id: UUID, data: NoteInput, user=Depends(current_user)):
    with connect() as conn:
        require_access(conn, investigation_id, user["id"], write=True)
        return conn.execute("INSERT INTO annotations(id,investigation_id,author_id,body) VALUES(%s,%s,%s,%s) RETURNING id",
                            (uuid4(), investigation_id, user["id"], data.body)).fetchone()


STATIC = Path(__file__).parent.parent / "static"
if STATIC.exists():
    app.mount("/assets", StaticFiles(directory=STATIC / "assets"), name="assets")


@app.get("/", include_in_schema=False)
def index():
    if not (STATIC / "index.html").exists():
        raise HTTPException(503, "Interface non construite")
    return FileResponse(STATIC / "index.html")
