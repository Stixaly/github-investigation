import json
from datetime import datetime, timezone
from uuid import uuid4

import pytest

from investigator.collector import FetchError, LeaseLost, guard, next_page
from investigator.db import connect
from investigator.legacy import import_snapshot
from investigator.store import ingest_github
from investigator.worker import claim
from tests.conftest import PASSWORD, investigation, login_as, scan

REPO = {'id': 123, 'full_name': 'alice/example', 'private': False,
        'owner': {'id': 42, 'login': 'alice', 'type': 'User'}}


def seed_data(scan_id, when=None, repo=None):
    with connect() as conn:
        return ingest_github(conn, scan_id, 'repository', repo or REPO, 'https://api.github.com/repos/alice/example',
                             when or datetime.now(timezone.utc))


def test_auth_and_csrf(client):
    client.headers.pop('X-CSRF-Token')
    assert client.post('/api/v1/investigations', json={'title': 'denied'}).status_code == 403
    client.cookies.clear()
    assert client.get('/api/v1/investigations').status_code == 401
    response = client.post('/api/v1/login', json={'login': 'alice', 'password': PASSWORD}, headers={'Origin': 'https://evil.test'})
    assert response.status_code == 403


def test_session_revocation(client):
    assert client.post('/api/v1/logout').status_code == 204
    assert client.get('/api/v1/me').status_code == 401


def test_login_rate_limit(client):
    for _ in range(5):
        assert client.post('/api/v1/login', json={'login': 'alice', 'password': 'wrong'}).status_code == 401
    assert client.post('/api/v1/login', json={'login': 'alice', 'password': PASSWORD}).status_code == 429


def test_private_investigation_is_not_visible(client):
    case = investigation(client)
    sid = scan(client, case)
    identity = seed_data(sid)
    graph = client.get(f'/api/v1/scans/{sid}/graph').json()
    proof = graph['nodes'][0]['evidence_hash']
    login_as(client, 'bob')
    assert client.get('/api/v1/investigations').json() == []
    assert client.get(f'/api/v1/scans/{sid}/graph').status_code == 404
    assert client.get(f'/api/v1/evidence/{proof}').status_code == 404
    assert client.get(f'/api/v1/entities/{identity}/overlaps').json() == []
    assert client.get(f'/api/v1/investigations/{case}/notes').status_code == 404


def test_reader_and_revocation(client):
    case = investigation(client)
    assert client.put(f'/api/v1/investigations/{case}/members', json={'login': 'reader', 'role': 'reader'}).status_code == 200
    login_as(client, 'reader')
    assert len(client.get('/api/v1/investigations').json()) == 1
    assert client.post(f'/api/v1/investigations/{case}/scans', json={'seed': 'alice'}).status_code == 403
    assert client.post(f'/api/v1/investigations/{case}/notes', json={'body': 'forbidden'}).status_code == 403
    login_as(client, 'alice')
    assert client.delete(f'/api/v1/investigations/{case}/members/reader').status_code == 204
    login_as(client, 'reader')
    assert client.get(f'/api/v1/investigations/{case}/scans').status_code == 404


def test_idempotency_and_pending_budget(client):
    case = investigation(client)
    body = {'seed': 'alice/example', 'idempotency_key': str(uuid4())}
    url = f'/api/v1/investigations/{case}/scans'
    first = client.post(url, json=body).json()['id']
    assert client.post(url, json=body).json()['id'] == first
    assert client.post(url, json={**body, 'seed': 'other/repo'}).status_code == 409
    scan(client, case)
    scan(client, case)
    assert client.post(url, json={'seed': 'another/repo'}).status_code == 429


def test_canonical_entities_and_temporal_labels(client):
    case = investigation(client)
    old, new = scan(client, case), scan(client, case)
    when = datetime(2026, 6, 26, tzinfo=timezone.utc)
    first_id = seed_data(old, when)
    seed_data(old, when)
    assert seed_data(new, repo={**REPO, 'full_name': 'alice/renamed'}) == first_id
    graph = client.get(f'/api/v1/scans/{old}/graph').json()
    assert len(graph['nodes']) == 2 and len(graph['edges']) == 1
    repo = next(n for n in graph['nodes'] if n['kind'] == 'Repository')
    assert repo['label'] == 'alice/example'
    assert repo['observed_at'].startswith('2026-06-26')
    assert repo['recorded_at'] != repo['observed_at']
    filtered = client.get(f'/api/v1/scans/{old}/graph?q=example').json()
    assert [node['label'] for node in filtered['nodes']] == ['alice/example']
    assert client.get(f'/api/v1/scans/{old}/graph?q=renamed').json()['nodes'] == []


def test_comparison_not_deletion(client):
    case = investigation(client)
    old, new = scan(client, case), scan(client, case)
    seed_data(old)
    result = client.get(f'/api/v1/scans/{old}/compare/{new}').json()
    assert result['not_reobserved'] == 1 and result['newly_observed'] == 0
    assert 'supprimé' in result['notice']
    login_as(client, 'bob')
    assert client.get(f'/api/v1/scans/{old}/compare/{new}').status_code == 404


def test_private_payload_not_persisted(client):
    case = investigation(client)
    sid = scan(client, case)
    assert seed_data(sid, repo={**REPO, 'private': True}) is None
    with connect() as conn:
        ingest_github(conn, sid, 'events', {'public': False, 'secret': 'private-data'}, 'provider', datetime.now(timezone.utc))
        assert conn.execute('SELECT count(*) AS n FROM evidence').fetchone()['n'] == 0


def test_graph_bounds_and_evidence(client):
    case = investigation(client)
    sid = scan(client, case)
    seed_data(sid)
    result = client.get(f'/api/v1/scans/{sid}/graph?limit=1').json()
    assert len(result['nodes']) == 1 and result['truncated']
    assert client.get(f'/api/v1/evidence/{result["nodes"][0]["evidence_hash"]}').status_code == 200


def test_worker_fencing_and_cancel(client):
    case = investigation(client)
    sid = scan(client, case)
    job = claim()
    assert str(job['id']) == sid
    assert claim() is None
    assert client.post(f'/api/v1/scans/{sid}/cancel').status_code == 200
    with connect() as conn, pytest.raises(LeaseLost):
        guard(conn, job)


def test_legacy_import_is_idempotent_and_inert(client, tmp_path):
    case = investigation(client)
    (tmp_path / 'users_db.json').write_text(json.dumps({'alice': REPO['owner']}))
    (tmp_path / 'repos_db.json').write_text(json.dumps({'alice': [{'full_name': 'alice/old', 'fork': False}]}))
    (tmp_path / 'sets.pkl').write_bytes(b'not-a-pickle-never-read')
    result = import_snapshot(tmp_path, case, 'alice', '2026-06-26')
    again = import_snapshot(tmp_path, case, 'alice', '2026-06-26')
    assert again['scan_id'] == result['scan_id'] and again['reused']
    graph = client.get(f'/api/v1/scans/{result["scan_id"]}/graph').json()
    assert len(graph['edges']) == 1
    assert next(n for n in graph['nodes'] if n['kind'] == 'Repository')['provisional']
    assert all(n['time_precision'] == 'day' for n in graph['nodes'])


def test_pagination_origin():
    assert next_page('<https://api.github.com/repos/a/b/forks?page=2>; rel="next"',
                     'https://api.github.com/repos/a/b/forks?page=1').endswith('page=2')
    canonical = '<https://api.github.com/repositories/42/forks?page=2>; rel="next"'
    assert next_page(canonical, 'https://api.github.com/repos/a/b/forks', '/repositories/42/forks').endswith('page=2')
    with pytest.raises(FetchError):
        next_page(canonical, 'https://api.github.com/repos/a/b/forks', '/repositories/99/forks')
    with pytest.raises(FetchError):
        next_page('<https://evil.test/steal>; rel="next"', 'https://api.github.com/repos/a/b/forks')
