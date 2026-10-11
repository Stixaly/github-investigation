import logging
import time

from .collector import BudgetExceeded, FetchError, LeaseLost, RateLimited, collect
from .db import connect

log = logging.getLogger(__name__)


def claim():
    with connect() as conn:
        conn.execute("UPDATE scans SET status='failed',error='Limite de reprises atteinte',finished_at=now() "
                     "WHERE status='running' AND lease_until<now() AND attempts>=3")
        row = conn.execute(
            "SELECT id FROM scans WHERE source='github' AND available_at<=now() "
            "AND (status IN ('queued','waiting_quota') OR (status='running' AND lease_until<now())) "
            "ORDER BY requested_at FOR UPDATE SKIP LOCKED LIMIT 1"
        ).fetchone()
        if not row:
            return None
        return conn.execute(
            "UPDATE scans SET status='running',generation=generation+1,attempts=attempts+1,"
            "started_at=coalesce(started_at,now()),lease_until=now()+interval '90 seconds',error=NULL "
            "WHERE id=%s RETURNING *", (row["id"],)
        ).fetchone()


def execute(scan):
    delay = 0
    error = None
    try:
        status = collect(scan)
    except LeaseLost:
        return
    except RateLimited as exc:
        status, delay = "waiting_quota", exc.seconds
        error = "Attente du quota GitHub partagé"
    except BudgetExceeded:
        status, error = "partial", "Budget de requêtes atteint pendant la reprise"
    except FetchError as exc:
        status, error = "failed", str(exc)
    except Exception:
        # Avoid persisting provider payloads or credentials in client-visible errors.
        log.exception("Scan failed: %s", scan["id"])
        status, error = "failed", "Erreur interne de collecte ; consulter le journal du worker"
    with connect() as conn:
        conn.execute(
            "UPDATE scans SET status=%s,error=%s,lease_until=NULL,"
            "available_at=now()+(%s * interval '1 second'),"
            "finished_at=CASE WHEN %s='waiting_quota' THEN NULL ELSE now() END,"
            "attempts=CASE WHEN %s='waiting_quota' THEN greatest(attempts-1,0) ELSE attempts END "
            "WHERE id=%s AND status='running' AND generation=%s",
            (status, error, delay, status, status, scan["id"], scan["generation"])
        )


def main():
    logging.basicConfig(level=logging.INFO)
    while True:
        try:
            scan = claim()
            if scan:
                execute(scan)
            else:
                time.sleep(2)
        except KeyboardInterrupt:
            return
        except Exception:
            log.exception("Worker loop failed")
            time.sleep(5)


if __name__ == "__main__":
    main()
