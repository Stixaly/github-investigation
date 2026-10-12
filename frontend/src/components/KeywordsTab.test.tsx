import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Action } from '../hooks/useAction';
import { KeywordsTab } from './KeywordsTab';

const action = (): Action => {
  const setError = vi.fn();
  return {
    busy: false,
    error: '',
    setError,
    run: async (task) => {
      try {
        await task();
      } catch (failure) {
        setError((failure as Error).message);
      }
    },
  };
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('KeywordsTab', () => {
  it('tries a draft on a sample, then publishes it', async () => {
    const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith('/keyword-lists/preview'))
        return Response.json({
          engine_version: 1,
          failing_tests: [],
          results: [
            {
              rule: 'theme',
              outcome: 'match',
              review_priority: 'normal',
              hits: [
                { field: 'repository.name', term: 'malware', start: 8, end: 15, excerpt: 'analyse-malware' },
              ],
              exclusions: [],
              not_evaluated: ['repository.description'],
            },
          ],
        });
      if (init?.method === 'POST') return Response.json({ id: 'l1', version: 1 }, { status: 201 });
      return Response.json([]);
    });
    vi.stubGlobal('fetch', fetch);
    render(<KeywordsTab investigationId="i1" canWrite action={action()} />);
    expect(await screen.findByText('Aucune liste publiée dans ce dossier.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Nouvelle liste/ }));
    await userEvent.type(screen.getByLabelText('Nom du dépôt'), 'analyse-malware');
    await userEvent.click(screen.getByRole('button', { name: /Essayer/ }));
    expect(await screen.findByText('Correspondance')).toBeInTheDocument();
    expect(screen.getByText(/Non collecté : repository.description/)).toBeInTheDocument();
    const previewCall = fetch.mock.calls.find(([url]) => String(url).endsWith('/preview'));
    const sent = JSON.parse(String(previewCall?.[1]?.body));
    // Empty sample fields are left out: they count as not collected.
    expect(sent.input).toEqual({ 'repository.name': 'analyse-malware' });
    expect(sent.definition.format).toBe('observatory.keyword-list');
    await userEvent.click(screen.getByRole('button', { name: /Publier/ }));
    expect(await screen.findByText('Version 1 publiée.')).toBeInTheDocument();
    const publishCall = fetch.mock.calls.find(
      ([url, init]) => String(url).endsWith('/i1/keyword-lists') && init?.method === 'POST',
    );
    expect(publishCall).toBeDefined();
  });

  it('explains invalid JSON without calling the API, and hides editing from readers', async () => {
    const fetch = vi.fn(async (_input: RequestInfo | URL) => Response.json([]));
    vi.stubGlobal('fetch', fetch);
    const handlers = action();
    const { unmount } = render(<KeywordsTab investigationId="i1" canWrite action={handlers} />);
    await userEvent.click(await screen.findByRole('button', { name: /Nouvelle liste/ }));
    const editor = screen.getByLabelText('Définition de la liste');
    await userEvent.clear(editor);
    await userEvent.type(editor, 'pas du json');
    await userEvent.click(screen.getByRole('button', { name: /Essayer/ }));
    expect(handlers.setError).toHaveBeenCalledWith(expect.stringMatching(/^JSON invalide/));
    expect(fetch.mock.calls.filter(([url]) => String(url).endsWith('/preview'))).toHaveLength(0);
    unmount();
    render(<KeywordsTab investigationId="i1" canWrite={false} action={action()} />);
    expect(await screen.findByText('Aucune liste publiée dans ce dossier.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Nouvelle liste/ })).not.toBeInTheDocument();
  });
});
