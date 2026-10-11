import os

import pytest
from fastapi.testclient import TestClient

from investigator.api import app
from investigator.cli import create_user
from investigator.db import connect

PASSWORD = 'local-test-only-password-234!'


@pytest.fixture
def database():
    if os.environ.get('DB_HOST') != 'test-postgres':
        pytest.fail('Integration tests require the isolated test-postgres service')
    with connect() as conn:
        conn.execute('TRUNCATE users,investigations,scans,entities,evidence,relations,login_attempts RESTART IDENTITY CASCADE')
        conn.execute('UPDATE github_budget SET next_request_at=now()')
    for login in ('alice', 'bob', 'reader'):
        create_user(login, PASSWORD)


@pytest.fixture
def client(database):
    with TestClient(app, base_url='http://localhost:8088') as client:
        response = client.post('/api/v1/login', json={'login': 'alice', 'password': PASSWORD})
        assert response.status_code == 200
        client.headers['X-CSRF-Token'] = response.json()['csrf']
        yield client


def login_as(client, login):
    response = client.post('/api/v1/login', json={'login': login, 'password': PASSWORD})
    assert response.status_code == 200
    client.headers['X-CSRF-Token'] = response.json()['csrf']


def investigation(client, title='Investigation test'):
    response = client.post('/api/v1/investigations', json={'title': title})
    assert response.status_code == 201
    return response.json()['id']


def scan(client, investigation_id):
    response = client.post(f'/api/v1/investigations/{investigation_id}/scans', json={'seed': 'alice/example'})
    assert response.status_code == 202
    return response.json()['id']
