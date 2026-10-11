import pytest

from investigator import collector
from investigator.collector import BudgetExceeded, RateLimited, reserve_request
from investigator.db import connect
from investigator.worker import claim, execute
from tests.collector_cases import ROOT
from tests.conftest import investigation, scan


def test_offline_full_collection(client, monkeypatch):
    case = investigation(client)
    sid = scan(client, case)
    job = claim()

    def get(self, url):
        if url.endswith('alice/example'):
            return ROOT, ''
        if '/contributors?' in url:
            return [{'id': 3, 'login': 'bob'}], ''
        if '/issues?' in url:
            return [{'id': 4, 'number': 1, 'title': 'An issue', 'user': {'id': 3, 'login': 'bob'}}], ''
        if '/forks?' in url:
            return [{'id': 5, 'full_name': 'bob/example', 'private': False, 'owner': {'id': 3, 'login': 'bob'}}], ''
        return [{'id': '1234', 'public': True, 'type': 'WatchEvent', 'actor': {'id': 3, 'login': 'bob'},
                 'created_at': '2026-01-01T00:00:00Z'}], ''

    monkeypatch.setattr(collector.GitHub, 'get', get)
    execute(job)
    result = client.get(f'/api/v1/scans/{sid}/graph').json()
    assert {edge['kind'] for edge in result['edges']} == {'OWNS', 'FORK_OF', 'AUTHORED', 'IN_REPOSITORY', 'CONTRIBUTED_TO', 'STARRED'}
    assert next(e for e in result['edges'] if e['kind'] == 'STARRED')['source_event_at'].startswith('2026-01-01')
    assert result['coverage']['events']['status'] == 'windowed'


def test_budget_and_quota_are_persisted(client):
    case = investigation(client)
    sid = scan(client, case)
    job = claim()
    reserve_request(job)
    with pytest.raises(RateLimited):
        reserve_request(job)
    with connect() as conn:
        conn.execute('UPDATE scans SET requests_used=max_requests WHERE id=%s', (sid,))
    with pytest.raises(BudgetExceeded):
        reserve_request(job)


def test_rate_limit_requeues_without_losing_observations(client, monkeypatch):
    case = investigation(client)
    sid = scan(client, case)
    job = claim()

    def limited(self, url):
        if url.endswith('alice/example'):
            return ROOT, ''
        raise RateLimited(300)

    monkeypatch.setattr(collector.GitHub, 'get', limited)
    execute(job)
    with connect() as conn:
        row = conn.execute('SELECT status,available_at,requested_at FROM scans WHERE id=%s', (sid,)).fetchone()
        assert row['status'] == 'waiting_quota'
        assert row['available_at'] > row['requested_at']
    assert len(client.get(f'/api/v1/scans/{sid}/graph').json()['nodes']) == 2
