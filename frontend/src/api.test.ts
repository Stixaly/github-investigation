import { describe, expect, it, vi } from 'vitest';
import { api, setCsrfToken } from './api';

function stubFetch(status: number, body?: unknown) {
  const fetch = vi.fn(async () =>
    body === undefined ? new Response(null, { status }) : Response.json(body, { status }),
  );
  vi.stubGlobal('fetch', fetch);
  return fetch;
}

describe('api', () => {
  it('sends JSON with the CSRF token on the same origin', async () => {
    const fetch = stubFetch(200, { ok: true });
    setCsrfToken('token-1');
    await expect(api('/investigations', 'POST', { title: 'x' })).resolves.toEqual({ ok: true });
    expect(fetch).toHaveBeenCalledWith('/api/v1/investigations', {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'Content-Type': 'application/json', 'X-CSRF-Token': 'token-1' },
      body: '{"title":"x"}',
    });
  });

  it('returns nothing for 204', async () => {
    stubFetch(204);
    await expect(api('/logout', 'POST')).resolves.toBeUndefined();
  });

  it('surfaces the server detail', async () => {
    stubFetch(429, { detail: 'Trois scans sont déjà en attente ou actifs' });
    await expect(api('/investigations/1/scans', 'POST', {})).rejects.toThrow('Trois scans');
  });

  it('falls back when the error is not a message', async () => {
    stubFetch(422, { detail: [{ loc: ['body'] }] });
    await expect(api('/x')).rejects.toThrow('Requête incorrecte');
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('<html>', { status: 502 })),
    );
    await expect(api('/x')).rejects.toThrow('Service indisponible');
  });
});
