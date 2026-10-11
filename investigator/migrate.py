import hashlib
from pathlib import Path

from psycopg import sql

from .db import connect, secret


def main():
    source = Path(__file__).with_name("schema.sql").read_text()
    checksum = hashlib.sha256(source.encode()).hexdigest()
    with connect() as conn:
        conn.execute("SELECT pg_advisory_xact_lock(482760)")
        conn.execute("CREATE TABLE IF NOT EXISTS schema_migrations (version integer PRIMARY KEY, checksum text NOT NULL)")
        rows = conn.execute("SELECT * FROM schema_migrations ORDER BY version").fetchall()
        if rows and (len(rows) != 1 or rows[0]["version"] != 1 or rows[0]["checksum"] != checksum):
            raise RuntimeError("Incompatible migration history; do not alter an applied migration")
        if not rows:
            conn.execute(source)
            conn.execute("INSERT INTO schema_migrations VALUES (1,%s)", (checksum,))
        if not conn.execute("SELECT 1 FROM pg_roles WHERE rolname='investigator'").fetchone():
            conn.execute(sql.SQL("CREATE ROLE investigator LOGIN PASSWORD {}").format(sql.Literal(secret("APP_PASSWORD"))))
        conn.execute("REVOKE CREATE ON SCHEMA public FROM PUBLIC")
        conn.execute("GRANT USAGE ON SCHEMA public TO investigator")
        conn.execute("GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO investigator")
        conn.execute("REVOKE INSERT, UPDATE, DELETE ON schema_migrations FROM investigator")
        conn.execute("GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO investigator")
    print("Schema v1 verified")


if __name__ == "__main__":
    main()
