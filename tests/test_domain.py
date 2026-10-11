import pytest

from investigator.domain import parse_seed
from investigator.legacy import repo_items


@pytest.mark.parametrize('seed,kind,value', [
    ('octocat', 'Account', 'octocat'),
    ('https://github.com/octocat/Hello-World/', 'Repository', 'octocat/Hello-World'),
    ('owner/repo', 'Repository', 'owner/repo'),
])
def test_seed(seed, kind, value):
    assert parse_seed(seed) == (kind, value)


@pytest.mark.parametrize('seed', [
    'https://evil.example/u/r', 'https://github.com@evil.example/u/r',
    'https://github.com/u/r?q=secret', 'owner/../x', 'owner/..', '/etc/passwd',
    'http://github.com/u/r', 'https://github.com/u/r/tree/main',
])
def test_reject_unsafe_seed(seed):
    with pytest.raises(ValueError):
        parse_seed(seed)


def test_stream_repositories(tmp_path):
    path = tmp_path / 'repos.json'
    path.write_text('{"alice":[{"full_name":"alice/a","topics":["x"]},{"full_name":"alice/b"}],"bad":{"error":"404"}}')
    assert [(owner, repo['full_name']) for owner, repo in repo_items(path)] == [('alice', 'alice/a'), ('alice', 'alice/b')]
