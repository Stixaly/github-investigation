import time
from datetime import datetime, timezone
from urllib.parse import urlsplit

import httpx
from psycopg.types.json import Jsonb

from .db import connect, secret
from .domain import parse_seed
from .store import ingest_github

API = "https://api.github.com"


class LeaseLost(Exception):
    pass


class BudgetExceeded(Exception):
    pass


class RateLimited(Exception):
    def __init__(self, seconds):
        self.seconds = max(1, min(int(seconds), 86400))


class FetchError(Exception):
    pass


def guard(conn, scan):
    row = conn.execute("SELECT * FROM scans WHERE id=%s FOR UPDATE", (scan["id"],)).fetchone()
    if not row or row["status"] != "running" or row["generation"] != scan["generation"]:
        raise LeaseLost()
    conn.execute("UPDATE scans SET lease_until=now()+interval '90 seconds' WHERE id=%s", (scan["id"],))
    return row


def reserve_request(scan):
    with connect() as conn:
        row = guard(conn, scan)
        if row["requests_used"] >= row["max_requests"]:
            raise BudgetExceeded()
        budget = conn.execute("SELECT next_request_at FROM github_budget WHERE id=1 FOR UPDATE").fetchone()
        delay = (budget["next_request_at"] - datetime.now(timezone.utc)).total_seconds()
        if delay > 0:
            raise RateLimited(delay + 1)
        conn.execute("UPDATE github_budget SET next_request_at=now()+interval '1 second' WHERE id=1")
        conn.execute("UPDATE scans SET requests_used=requests_used+1 WHERE id=%s", (scan["id"],))


def next_page(link_header: str, current_path: str, canonical_path: str | None = None):
    for part in link_header.split(","):
        segments = part.split(";")
        if len(segments) < 2 or 'rel="next"' not in segments[1]:
            continue
        url = segments[0].strip().strip("<>")
        parsed = urlsplit(url)
        if (parsed.scheme != "https" or parsed.netloc != "api.github.com"
                or parsed.path not in {urlsplit(current_path).path, canonical_path}):
            raise FetchError("Lien de pagination non autorisé")
        return url
    return None


class GitHub:
    def __init__(self, scan):
        self.scan = scan
        token = secret("GITHUB_TOKEN")
        headers = {"Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28",
                   "User-Agent": "github-investigation/0.1"}
        if token:
            headers["Authorization"] = "Bearer " + token
        self.client = httpx.Client(headers=headers, timeout=20, follow_redirects=False, trust_env=False)

    def get(self, url):
        if not url.startswith(API + "/"):
            raise FetchError("Origine API non autorisée")
        # Smooth requests within a running scan. Longer quota waits are persisted by the worker.
        time.sleep(1.05)
        reserve_request(self.scan)
        start = time.monotonic()
        try:
            with self.client.stream("GET", url) as response:
                body = bytearray()
                for chunk in response.iter_bytes():
                    body.extend(chunk)
                    if len(body) > 2 * 1024 * 1024 or time.monotonic() - start > 30:
                        raise FetchError("Réponse trop volumineuse ou trop lente")
                code = response.status_code
                if response.headers.get("x-ratelimit-remaining") == "0":
                    try:
                        delay = max(float(response.headers.get("x-ratelimit-reset", "0")) - time.time(), 60)
                    except ValueError:
                        delay = 60
                    self.block(delay)
                    if code in (403, 429):
                        raise RateLimited(delay)
                if code == 429 or (code == 403 and "retry-after" in response.headers):
                    try:
                        delay = float(response.headers.get("retry-after", "60"))
                    except ValueError:
                        delay = 60
                    self.block(delay)
                    raise RateLimited(delay)
                if code != 200:
                    raise FetchError(f"HTTP {code} : ressource non disponible (aucune suppression inférée)")
                try:
                    payload = httpx.Response(200, content=bytes(body)).json()
                except ValueError as exc:
                    raise FetchError("Réponse JSON invalide") from exc
                return payload, response.headers.get("link", "")
        except httpx.HTTPError as exc:
            raise FetchError("Échec réseau ; collecte incomplète") from exc

    @staticmethod
    def block(seconds):
        with connect() as conn:
            conn.execute("UPDATE github_budget SET next_request_at=greatest(next_request_at,now()+(%s * interval '1 second')) "
                         "WHERE id=1", (min(max(seconds, 1), 86400),))


def collect(scan):
    kind, seed = parse_seed(scan["seed"])
    github = GitHub(scan)
    coverage = dict(scan["coverage"])
    root_url = API + ("/repos/" if kind == "Repository" else "/users/") + seed
    try:
        root, _ = github.get(root_url)
        if not isinstance(root, dict):
            raise FetchError("Métadonnées invalides")
        if root.get("private") or root.get("visibility", "public") != "public":
            raise FetchError("Les dépôts privés sont hors périmètre")
        with connect() as conn:
            guard(conn, scan)
            root_id = ingest_github(conn, scan["id"], "repository" if kind == "Repository" else "account",
                                    root, root_url, datetime.now(timezone.utc))
        if not root_id:
            raise FetchError("Identité GitHub manquante")
        coverage["seed"] = {"status": "complete", "items": 1}
        if kind == "Repository":
            plan = [("contributors", root_url + "/contributors?per_page=50"),
                    ("forks", root_url + "/forks?per_page=50"),
                    ("issues", root_url + "/issues?state=all&per_page=50"),
                    ("events", root_url + "/events?per_page=50")]
            coverage["stargazers"] = {"status": "not_requested", "reason": "Listings restreints ; événements partiels seulement"}
            coverage["watchers"] = {"status": "not_requested", "reason": "Connecteur à qualifier"}
        else:
            suffix = "/orgs/" if root.get("type") == "Organization" else "/users/"
            plan = [("repositories", API + suffix + seed + "/repos?type=owner&per_page=50")]
        for role, url in plan:
            result = {"status": "partial", "items": 0, "pages": 0}
            coverage[role] = result
            try:
                for _ in range(2):
                    payload, link = github.get(url)
                    if not isinstance(payload, list):
                        raise FetchError("Liste attendue")
                    observed = datetime.now(timezone.utc)
                    with connect() as conn:
                        guard(conn, scan)
                        for item in payload:
                            if not isinstance(item, dict):
                                raise FetchError("Objet GitHub invalide")
                            ingest_github(conn, scan["id"], role, item, url, observed, root_id)
                    result["items"] += len(payload)
                    result["pages"] += 1
                    # GitHub can paginate /repos/owner/name using /repositories/<stable-id>.
                    canonical = None
                    if kind == "Repository" and str(root.get("id", "")).isdigit():
                        canonical = f"/repositories/{root['id']}/{role}"
                    following = next_page(link, url, canonical)
                    if not following:
                        result["status"] = "windowed" if role == "events" else "complete"
                        break
                    url = following
                if result["status"] == "partial":
                    result["reason"] = "Plafond de deux pages par endpoint atteint"
                if role == "events":
                    result["reason"] = "Flux partiel : au plus 300 événements / 30 jours, deux pages collectées au maximum"
            except BudgetExceeded:
                result["reason"] = "Budget de requêtes atteint"
            except FetchError as exc:
                result.update(status="error", reason=str(exc))
            finally:
                with connect() as conn:
                    guard(conn, scan)
                    conn.execute("UPDATE scans SET coverage=%s WHERE id=%s", (Jsonb(coverage), scan["id"]))
        # 'succeeded' means the requested bounded profile completed, not exhaustive GitHub coverage.
        partial = any(x["status"] in ("partial", "error") for x in coverage.values())
        return "partial" if partial else "succeeded"
    finally:
        github.client.close()
