import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Action } from '../hooks/useAction';
import { DiscoveryPanel } from './DiscoveryPanel';

const action = (): Action => ({ busy: false, error: '', setError: vi.fn(), run: async (task) => task() });

const preview = {
  label: 'Dépôts : loader fork:true',
  max_requests_upper_bound: 18,
  resource: 'search',
  plan: {
    kind: 'repositories',
    queries: [{ q: 'loader fork:true', base: 'loader fork:true', origin: 'requête saisie' }],
    max_pages: 2,
    max_subdivisions: 4,
    include_forks: true,
    local_checks: [],
    warnings: ['GitHub ne donne accès qu’aux 1000 premiers résultats d’une recherche.'],
  },
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('DiscoveryPanel', () => {
  it('shows the plan before launching it, and forgets it when the query changes', async () => {
    const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith('/discoveries/preview')) return Response.json(preview);
      if (url.endsWith('/discoveries') && init?.method === 'POST')
        return Response.json({ id: 's1', seed: preview.label, status: 'queued' }, { status: 202 });
      return Response.json([]);
    });
    vi.stubGlobal('fetch', fetch);
    const onScanCreated = vi.fn();
    render(<DiscoveryPanel investigationId="i1" canWrite action={action()} onScanCreated={onScanCreated} />);
    await userEvent.click(screen.getByText('Découvrir sur GitHub'));
    const launch = screen.getByRole('button', { name: /Lancer la découverte/ });
    expect(launch).toBeDisabled();
    await userEvent.type(screen.getByLabelText('Requête GitHub'), 'loader');
    await userEvent.click(screen.getByRole('button', { name: /Voir le plan/ }));
    expect(await screen.findByText('loader fork:true')).toBeInTheDocument();
    expect(screen.getByText(/1000 premiers résultats/)).toBeInTheDocument();
    expect(launch).toBeEnabled();
    const sent = JSON.parse(
      String(fetch.mock.calls.find(([url]) => String(url).endsWith('/preview'))?.[1]?.body),
    );
    expect(sent).toEqual({
      kind: 'repositories',
      query: 'loader',
      include_forks: true,
      max_pages: 2,
      max_requests: 12,
    });

    await userEvent.type(screen.getByLabelText('Requête GitHub'), 's');
    expect(screen.queryByText('loader fork:true')).not.toBeInTheDocument();
    expect(launch).toBeDisabled();
    await userEvent.click(screen.getByRole('button', { name: /Voir le plan/ }));
    await userEvent.click(await screen.findByRole('button', { name: /Lancer la découverte/ }));
    expect(onScanCreated).toHaveBeenCalledWith([expect.objectContaining({ id: 's1' })]);
  });

  it('searches code from a typed query only, without forks by default', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (_input: RequestInfo | URL) => Response.json([])),
    );
    render(
      <DiscoveryPanel investigationId="i1" canWrite={false} action={action()} onScanCreated={vi.fn()} />,
    );
    await userEvent.click(screen.getByText('Découvrir sur GitHub'));
    await userEvent.selectOptions(screen.getByLabelText('Type de recherche'), 'code');
    expect(screen.getByRole('option', { name: 'D’une liste publiée' })).toBeDisabled();
    expect(screen.getByRole('checkbox')).not.toBeChecked();
  });
});
