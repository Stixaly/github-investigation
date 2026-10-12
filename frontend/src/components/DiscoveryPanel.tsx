import { useEffect, useState } from 'react';
import { Rocket, Telescope } from 'lucide-react';
import { api } from '../api';
import type { Action } from '../hooks/useAction';
import type { DiscoveryPreview, KeywordListSummary, Scan } from '../types';

type Props = {
  investigationId: string;
  canWrite: boolean;
  action: Action;
  onScanCreated: (scans: Scan[]) => void;
};

type Kind = 'repositories' | 'code';
type Source = 'query' | 'list';

/** Search GitHub for repositories or code: the plan is shown before anything is sent. */
export function DiscoveryPanel({ investigationId, canWrite, action, onScanCreated }: Props) {
  const { busy, run } = action;
  const [kind, setKind] = useState<Kind>('repositories');
  const [source, setSource] = useState<Source>('query');
  const [query, setQuery] = useState('');
  const [listChoice, setListChoice] = useState('');
  const [includeForks, setIncludeForks] = useState(true);
  const [maxPages, setMaxPages] = useState(2);
  const [budget, setBudget] = useState(12);
  const [lists, setLists] = useState<KeywordListSummary[]>([]);
  const [preview, setPreview] = useState<DiscoveryPreview | null>(null);

  useEffect(() => {
    let live = true;
    api<KeywordListSummary[]>(`/investigations/${investigationId}/keyword-lists`)
      .then((rows) => live && setLists(rows))
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [investigationId]);

  // Any change invalidates the plan: what is launched is always what was shown.
  const change =
    <T,>(setter: (value: T) => void) =>
    (value: T) => {
      setter(value);
      setPreview(null);
    };
  const body = () => {
    const [id, version] = listChoice.split(':');
    return {
      kind,
      ...(source === 'query' ? { query } : { list: { id, version: Number(version) } }),
      include_forks: includeForks,
      max_pages: maxPages,
      max_requests: budget,
    };
  };
  const showPlan = () =>
    run(async () => {
      setPreview(
        await api<DiscoveryPreview>(`/investigations/${investigationId}/discoveries/preview`, 'POST', body()),
      );
    });
  const launch = () =>
    run(async () => {
      const scan = await api<Scan>(`/investigations/${investigationId}/discoveries`, 'POST', {
        ...body(),
        idempotency_key: crypto.randomUUID(),
      });
      setPreview(null);
      onScanCreated([scan]);
    });
  const ready = source === 'query' ? query.trim().length > 0 : listChoice !== '';

  return (
    <details className="discovery">
      <summary>
        <Telescope size={16} /> Découvrir sur GitHub
      </summary>
      <div className="discovery-form">
        <label>
          Rechercher
          <select
            aria-label="Type de recherche"
            value={kind}
            onChange={(event) => {
              const next = event.target.value as Kind;
              change(setKind)(next);
              if (next === 'code') {
                setSource('query');
                setIncludeForks(false);
              }
            }}
          >
            <option value="repositories">Des dépôts</option>
            <option value="code">Du code (token requis)</option>
          </select>
        </label>
        <label>
          À partir
          <select
            aria-label="Source des termes"
            value={source}
            onChange={(event) => change(setSource)(event.target.value as Source)}
          >
            <option value="query">D’une requête GitHub</option>
            <option value="list" disabled={kind === 'code' || lists.length === 0}>
              D’une liste publiée
            </option>
          </select>
        </label>
        {source === 'query' ? (
          <label className="discovery-query">
            Requête
            <input
              aria-label="Requête GitHub"
              value={query}
              maxLength={256}
              placeholder={kind === 'code' ? '"VirtualAllocEx" language:C' : 'loader topic:windows'}
              onChange={(event) => change(setQuery)(event.target.value)}
            />
          </label>
        ) : (
          <label className="discovery-query">
            Liste
            <select
              aria-label="Liste de termes"
              value={listChoice}
              onChange={(event) => change(setListChoice)(event.target.value)}
            >
              <option value="">Choisir une liste</option>
              {lists.map((list) => (
                <option key={list.id} value={`${list.id}:${list.version}`}>
                  {list.name} · v{list.version}
                </option>
              ))}
            </select>
          </label>
        )}
        <label className="checkbox">
          <input
            type="checkbox"
            checked={includeForks}
            onChange={(event) => change(setIncludeForks)(event.target.checked)}
          />
          Inclure les forks
        </label>
        <label>
          Pages par requête
          <select
            aria-label="Pages par requête"
            value={maxPages}
            onChange={(event) => change(setMaxPages)(Number(event.target.value))}
          >
            {[1, 2, 5, 10].map((pages) => (
              <option key={pages} value={pages}>
                {pages} × 100 résultats
              </option>
            ))}
          </select>
        </label>
        <label>
          Budget
          <select
            aria-label="Budget de la découverte"
            value={budget}
            onChange={(event) => change(setBudget)(Number(event.target.value))}
          >
            {[6, 12, 24, 50].map((value) => (
              <option key={value} value={value}>
                {value} requêtes
              </option>
            ))}
          </select>
        </label>
        <div className="row-actions">
          <button className="secondary" disabled={busy || !ready} onClick={() => void showPlan()}>
            <Telescope size={15} /> Voir le plan
          </button>
          <button disabled={busy || !canWrite || !preview} onClick={() => void launch()}>
            <Rocket size={15} /> Lancer la découverte
          </button>
        </div>
      </div>
      {preview && (
        <div className="discovery-plan" aria-label="Plan de la découverte">
          <p>
            <strong>{preview.plan.queries.length}</strong> requête{preview.plan.queries.length > 1 ? 's' : ''}{' '}
            · au plus {preview.max_requests_upper_bound} appels avec les subdivisions · quota «{' '}
            {preview.resource} » · le budget de {budget} requêtes arrête la découverte avant.
          </p>
          <ol>
            {preview.plan.queries.map((planned) => (
              <li key={planned.q}>
                <code>{planned.q}</code> <small>{planned.origin}</small>
              </li>
            ))}
          </ol>
          {preview.plan.local_checks.length > 0 && (
            <>
              <h4>Vérifié localement ensuite</h4>
              <ul>
                {preview.plan.local_checks.map((check) => (
                  <li key={check}>{check}</li>
                ))}
              </ul>
            </>
          )}
          {preview.plan.warnings.map((warning) => (
            <p className="notice" key={warning}>
              {warning}
            </p>
          ))}
        </div>
      )}
    </details>
  );
}
