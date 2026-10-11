import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Found } from '../types';
import { Sidebar } from './Sidebar';

const props = () => ({
  user: { id: 'u1', login: 'analyste', csrf: 'token' },
  investigations: [{ id: 'i1', title: 'Dossier', description: '', role: 'owner' as const, scan_count: 1 }],
  activeId: 'i1',
  onSelect: vi.fn(),
  onCreate: vi.fn(),
  onLogout: vi.fn(),
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('Sidebar', () => {
  it('searches every investigation and opens the one that observed the entity', async () => {
    const found: Found[] = [
      {
        id: 'e1',
        kind: 'Account',
        label: 'demo-alice',
        investigations: [{ id: 'i2', title: 'Autre dossier' }],
      },
    ];
    const fetch = vi.fn(async (_input: RequestInfo | URL) => Response.json(found));
    vi.stubGlobal('fetch', fetch);
    const handlers = props();
    render(<Sidebar {...handlers} />);
    await userEvent.type(screen.getByLabelText('Rechercher dans tous les dossiers'), 'alice');
    expect(await screen.findByText('demo-alice')).toBeInTheDocument();
    expect(screen.getByText('Compte')).toBeInTheDocument();
    expect(String(fetch.mock.calls.at(-1)?.[0])).toBe('/api/v1/search?q=alice');
    await userEvent.click(screen.getByRole('button', { name: 'Autre dossier' }));
    expect(handlers.onSelect).toHaveBeenCalledWith('i2');
    expect(screen.getByLabelText('Rechercher dans tous les dossiers')).toHaveValue('');
  });

  it('says when nothing was observed, and does not search a single letter', async () => {
    const fetch = vi.fn(async (_input: RequestInfo | URL) => Response.json([]));
    vi.stubGlobal('fetch', fetch);
    render(<Sidebar {...props()} />);
    const input = screen.getByLabelText('Rechercher dans tous les dossiers');
    await userEvent.type(input, 'z');
    await new Promise((resolve) => setTimeout(resolve, 400));
    expect(fetch).not.toHaveBeenCalled();
    await userEvent.type(input, 'z');
    expect(await screen.findByText(/Aucune entité observée/)).toBeInTheDocument();
  });
});
