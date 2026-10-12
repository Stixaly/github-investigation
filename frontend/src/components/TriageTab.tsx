import { useCallback, useEffect, useState } from 'react';
import { api } from '../api';
import { formatDate, kindLabel } from '../format';
import type { Action } from '../hooks/useAction';
import type { TriagePage, TriageState } from '../types';

type Props = {
  investigationId: string;
  canWrite: boolean;
  action: Action;
};

export const triageLabel: Record<TriageState, string> = {
  new: 'Nouveau',
  in_review: 'En cours d’examen',
  retained: 'Retenu',
  dismissed: 'Écarté',
};

const STATES: TriageState[] = ['new', 'in_review', 'retained', 'dismissed'];

/** Matches of the investigation's keyword lists, one task per entity and rule, with the analyst's decision. */
export function TriageTab({ investigationId, canWrite, action }: Props) {
  const { busy, run, setError } = action;
  const [filter, setFilter] = useState<TriageState | ''>('new');
  const [page, setPage] = useState(1);
  const [data, setData] = useState<TriagePage | null>(null);
  const [notes, setNotes] = useState<Record<string, string>>({});

  const load = useCallback(
    () =>
      api<TriagePage>(
        `/investigations/${investigationId}/triage?page=${page}` + (filter ? `&state=${filter}` : ''),
      ).then(setData),
    [investigationId, filter, page],
  );
  useEffect(() => {
    load().catch((failure: Error) => setError(failure.message));
  }, [load, setError]);

  const decide = (id: string, state: TriageState) =>
    run(async () => {
      await api(`/triage/${id}`, 'PUT', { state, ...(notes[id] !== undefined ? { note: notes[id] } : {}) });
      await load();
    });

  const total = data ? Object.values(data.counts).reduce((sum, count) => sum + count, 0) : 0;
  return (
    <section className="panel triage">
      <h2>Triage</h2>
      <p>{data?.notice ?? 'Les correspondances des listes de termes, à examiner une par une.'}</p>
      <div className="triage-filters" role="group" aria-label="Filtrer par état">
        {STATES.map((state) => (
          <button
            key={state}
            className={filter === state ? 'chip selected' : 'chip'}
            aria-pressed={filter === state}
            onClick={() => {
              setFilter(state);
              setPage(1);
            }}
          >
            {triageLabel[state]} <span>{data?.counts[state] ?? 0}</span>
          </button>
        ))}
        <button
          className={filter === '' ? 'chip selected' : 'chip'}
          aria-pressed={filter === ''}
          onClick={() => {
            setFilter('');
            setPage(1);
          }}
        >
          Tous <span>{total}</span>
        </button>
      </div>
      {data && data.items.length === 0 && (
        <p>Aucune tâche dans cet état. Évaluez une liste depuis l’onglet « Termes ».</p>
      )}
      {data?.items.map((item) => (
        <article className="triage-item" key={item.id}>
          <header>
            <h3>{item.entity_label}</h3>
            <small>{kindLabel[item.entity_kind] ?? item.entity_kind}</small>
            <span className={'badge ' + item.state}>{triageLabel[item.state]}</span>
            {item.review_priority === 'high' && <span className="badge failed">Priorité haute</span>}
          </header>
          <p className="triage-rule">
            {item.list_name} v{item.list_version} · règle « {item.rule_id} » · observé le{' '}
            {formatDate(item.observed_at)} · évalué le {formatDate(item.evaluated_at)} (moteur v
            {item.engine_version})
          </p>
          <ul>
            {item.hits.map((hit, index) => (
              <li key={index}>
                « {hit.term} » dans {hit.field} : <q>{hit.excerpt}</q>
              </li>
            ))}
          </ul>
          {item.not_evaluated.length > 0 && <p>Non collecté : {item.not_evaluated.join(', ')}</p>}
          {item.updated_by && (
            <p className="triage-rule">
              Décision de {item.updated_by} le {formatDate(item.updated_at)}
              {item.note && ` : ${item.note}`}
            </p>
          )}
          {canWrite && (
            <div className="row-actions">
              <input
                aria-label={`Note sur ${item.entity_label}`}
                placeholder="Note (facultative)"
                maxLength={2000}
                value={notes[item.id] ?? ''}
                onChange={(event) => setNotes({ ...notes, [item.id]: event.target.value })}
              />
              {STATES.filter((state) => state !== item.state).map((state) => (
                <button
                  key={state}
                  className="secondary"
                  disabled={busy}
                  onClick={() => void decide(item.id, state)}
                >
                  {state === 'new' ? 'Rouvrir' : triageLabel[state]}
                </button>
              ))}
            </div>
          )}
        </article>
      ))}
      {data && (page > 1 || data.items.length === data.page_size) && (
        <div className="row-actions">
          <button className="text-button" disabled={page === 1} onClick={() => setPage(page - 1)}>
            Page précédente
          </button>
          <span>Page {page}</span>
          <button
            className="text-button"
            disabled={data.items.length < data.page_size}
            onClick={() => setPage(page + 1)}
          >
            Page suivante
          </button>
        </div>
      )}
    </section>
  );
}
