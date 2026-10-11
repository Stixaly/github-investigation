import argparse
import getpass
import json
import sys
from datetime import datetime, timezone
from uuid import uuid4

from .db import connect
from .legacy import import_snapshot
from .security import PASSWORDS, normalize_login
from .store import ingest_github


def create_user(login, password):
    login = normalize_login(login)
    if len(password) < 14 or len(password) > 1024:
        raise ValueError("Le mot de passe doit contenir entre 14 et 1024 caractères")
    with connect() as conn:
        conn.execute("INSERT INTO users(id,login,password_hash) VALUES(%s,%s,%s)", (uuid4(), login, PASSWORDS.hash(password)))


def demo(login):
    with connect() as conn:
        user = conn.execute("SELECT id FROM users WHERE login=%s", (login,)).fetchone()
        if not user:
            raise ValueError("Compte analyste introuvable")
        investigation_id = uuid4()
        conn.execute("INSERT INTO investigations(id,title,description) VALUES(%s,%s,%s)",
                     (investigation_id, "Terrain de démonstration", "Données fictives pour explorer l'interface et la comparaison temporelle."))
        conn.execute("INSERT INTO memberships VALUES(%s,%s,'owner')", (investigation_id, user["id"]))
        for month in (9, 10):
            scan_id = uuid4()
            at = datetime(2026, month, 1, tzinfo=timezone.utc)
            conn.execute("INSERT INTO scans(id,investigation_id,requested_by,source,seed,status,idempotency_key,coverage,"
                         "requested_at,started_at,finished_at) "
                         "VALUES(%s,%s,%s,'demo','demo-lab/observatory','succeeded',%s,%s::jsonb,%s,%s,%s)",
                         (scan_id, investigation_id, user["id"], uuid4(), '{"demo":{"status":"synthetic"}}', at, at, at))
            repo = {"id": "demo:repository:1", "full_name": "demo-lab/observatory", "description": "Corpus fictif d'analyse",
                    "owner": {"id": "demo:account:1", "login": "demo-lab", "type": "Organization"}, "private": False}
            root = ingest_github(conn, scan_id, "repository", repo, "demo:synthetic", at)
            for i, label in enumerate(("demo-alice", "demo-bob", "demo-charlie", "demo-delta", "demo-echo"), 2):
                person = {"id": f"demo:account:{i}", "login": label, "type": "User"}
                ingest_github(conn, scan_id, "contributors", person, "demo:synthetic", at, root)
                fork = {"id": f"demo:repository:{i}", "full_name": label + "/observatory", "owner": person, "fork": True, "private": False}
                ingest_github(conn, scan_id, "forks", fork, "demo:synthetic", at, root)
            if month == 10:
                ingest_github(conn, scan_id, "issues", {"id": "demo:issue:1", "number": 12,
                              "title": "Discussion d'architecture", "user": {"id": "demo:account:9", "login": "demo-foxtrot"},
                              "created_at": at.isoformat()}, "demo:synthetic", at, root)
    return str(investigation_id)


def main():
    parser = argparse.ArgumentParser(description="Administration locale de GitHub Investigation")
    sub = parser.add_subparsers(dest="command", required=True)
    create = sub.add_parser("create-user")
    create.add_argument("login")
    create.add_argument("--password-stdin", action="store_true")
    sample = sub.add_parser("demo")
    sample.add_argument("--login", required=True)
    legacy = sub.add_parser("import-legacy")
    legacy.add_argument("directory")
    legacy.add_argument("--investigation", required=True)
    legacy.add_argument("--login", required=True)
    legacy.add_argument("--observed-date", help="YYYY-MM-DD ; omettre si inconnu")
    args = parser.parse_args()
    if args.command == "create-user":
        password = sys.stdin.readline().rstrip("\r\n") if args.password_stdin else getpass.getpass("Mot de passe (14 caractères minimum) : ")
        create_user(args.login, password)
        print("Compte créé")
    elif args.command == "demo":
        print(demo(args.login))
    else:
        print(json.dumps(import_snapshot(args.directory, args.investigation, args.login, args.observed_date)))


if __name__ == "__main__":
    main()
