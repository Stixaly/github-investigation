// Same-origin JSON client. The session lives in an HttpOnly cookie; every
// modifying request also carries the CSRF token returned by /login and /me.

let csrfToken = '';

export function setCsrfToken(token: string) {
  csrfToken = token;
}

type Method = 'GET' | 'POST' | 'PUT' | 'DELETE';

export async function api<T>(path: string, method: Method = 'GET', body?: unknown): Promise<T> {
  const response = await fetch('/api/v1' + path, {
    method,
    credentials: 'same-origin',
    headers: { 'Content-Type': 'application/json', 'X-CSRF-Token': csrfToken },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) {
    const data = await response.json().catch(() => ({ detail: 'Service indisponible' }));
    throw new Error(typeof data?.detail === 'string' ? data.detail : 'Requête incorrecte');
  }
  return response.status === 204 ? (undefined as T) : response.json();
}
