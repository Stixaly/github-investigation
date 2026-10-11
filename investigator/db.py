import os
from pathlib import Path

import psycopg
from psycopg.rows import dict_row


def secret(name: str, default: str = "") -> str:
    path = os.environ.get(f"{name}_FILE")
    return Path(path).read_text().strip() if path else os.environ.get(name, default)


def connect():
    return psycopg.connect(
        host=os.environ.get("DB_HOST", "postgres"),
        dbname=os.environ.get("DB_NAME", "investigation"),
        user=os.environ.get("DB_USER", "investigator"),
        password=secret("DB_PASSWORD"),
        connect_timeout=5,
        row_factory=dict_row,
        options="-c timezone=UTC -c statement_timeout=15000 -c lock_timeout=5000",
    )
