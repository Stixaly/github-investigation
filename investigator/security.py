import hashlib
import hmac
import os
import re
import secrets
from datetime import datetime, timedelta, timezone
from urllib.parse import urlsplit

from fastapi import HTTPException, Request
from pwdlib import PasswordHash

from .db import connect

PASSWORDS = PasswordHash.recommended()
DUMMY_HASH = PASSWORDS.hash("unused-dummy-password-for-timing")
COOKIE = "investigation_session"
ORIGIN = os.environ.get("PUBLIC_ORIGIN", "http://localhost:8088").rstrip("/")
SECURE = ORIGIN.startswith("https://")


def check_origin(request: Request):
    if request.headers.get("origin") not in (None, ORIGIN):
        raise HTTPException(403, "Origine refusée")


def validate_origin():
    url = urlsplit(ORIGIN)
    if url.scheme not in ("http", "https") or not url.hostname or url.path or url.query or url.fragment:
        raise RuntimeError("PUBLIC_ORIGIN must be an origin, without path")
    if not SECURE and url.hostname not in ("localhost", "127.0.0.1", "::1"):
        raise RuntimeError("HTTPS is mandatory except on loopback")


def token_hash(token: str) -> str:
    return hashlib.sha256(token.encode()).hexdigest()


def csrf(token: str) -> str:
    return hmac.new(token.encode(), b"investigation-csrf-v1", hashlib.sha256).hexdigest()


def normalize_login(login: str) -> str:
    if not re.fullmatch(r"[a-zA-Z0-9_.-]{3,40}", login):
        raise ValueError("Login : 3–40 caractères alphanumériques, point, tiret ou underscore")
    return login.lower()


def authenticate(login: str, password: str):
    login = login.lower()
    # Commit unsuccessful attempts too: do not raise inside this transaction.
    with connect() as conn:
        conn.execute("SELECT pg_advisory_xact_lock(hashtext(%s))", ("login:" + login,))
        conn.execute("DELETE FROM login_attempts WHERE attempted_at < now() - interval '15 minutes'")
        count = conn.execute("SELECT count(*) AS n FROM login_attempts WHERE login=%s", (login,)).fetchone()["n"]
        if count >= 5:
            raise HTTPException(429, "Trop de tentatives ; réessayer dans 15 minutes")
        user = conn.execute("SELECT * FROM users WHERE login=%s", (login,)).fetchone()
        valid = PASSWORDS.verify(password, user["password_hash"] if user else DUMMY_HASH)
        if not user or not valid:
            conn.execute("INSERT INTO login_attempts(login) VALUES(%s)", (login,))
            result = None
        else:
            conn.execute("DELETE FROM login_attempts WHERE login=%s", (login,))
            token = secrets.token_urlsafe(32)
            expires = datetime.now(timezone.utc) + timedelta(hours=8)
            conn.execute("DELETE FROM sessions WHERE expires_at < now()")
            conn.execute("INSERT INTO sessions VALUES(%s,%s,%s)", (token_hash(token), user["id"], expires))
            conn.execute("INSERT INTO audit_events(actor_id,action) VALUES(%s,'login')", (user["id"],))
            result = (user, token)
    if result is None:
        raise HTTPException(401, "Identifiants incorrects")
    return result


def current_user(request: Request):
    token = request.cookies.get(COOKIE, "")
    with connect() as conn:
        user = conn.execute(
            "SELECT u.id,u.login FROM users u JOIN sessions s ON s.user_id=u.id "
            "WHERE s.token_hash=%s AND s.expires_at>now()", (token_hash(token),)
        ).fetchone()
    if not user:
        raise HTTPException(401, "Connexion requise")
    if request.method not in ("GET", "HEAD", "OPTIONS"):
        check_origin(request)
        if not hmac.compare_digest(request.headers.get("x-csrf-token", ""), csrf(token)):
            raise HTTPException(403, "Jeton CSRF incorrect")
    return user


def require_access(conn, investigation_id, user_id, write=False, owner=False):
    membership = conn.execute(
        "SELECT role FROM memberships WHERE investigation_id=%s AND user_id=%s",
        (investigation_id, user_id),
    ).fetchone()
    if not membership:
        raise HTTPException(404, "Investigation introuvable")
    if (write and membership["role"] == "reader") or (owner and membership["role"] != "owner"):
        raise HTTPException(403, "Droits insuffisants")
    return membership["role"]


def accessible_scan(conn, scan_id, user_id, write=False):
    scan = conn.execute("SELECT * FROM scans WHERE id=%s", (scan_id,)).fetchone()
    if not scan:
        raise HTTPException(404, "Scan introuvable")
    require_access(conn, scan["investigation_id"], user_id, write=write)
    return scan
