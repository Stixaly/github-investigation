import { useCallback, useEffect, useState } from 'react';
import { Download, FlaskConical, ListChecks, Pencil, Plus, Send } from 'lucide-react';
import { api } from '../api';
import { formatDate } from '../format';
import type { Action } from '../hooks/useAction';
import type { KeywordListSummary, KeywordVersion, ListPreview, RuleOutcome, RuleRun } from '../types';

type Props = {
  investigationId: string;
  canWrite: boolean;
  action: Action;
};

/** Smallest list that publishes: one rule, one test. */
export const TEMPLATE = {
  format: 'observatory.keyword-list',
  schema_version: 1,
  name: 'Nouvelle liste',
  description: 'Une correspondance signale un sujet à examiner, jamais un dépôt malveillant.',
  rules: [
    {
      id: 'theme',
      name: 'Thème',
      review_priority: 'normal',
      any: [
        {
          fields: ['repository.name', 'repository.description', 'repository.topics'],
          match: 'whole_word',
          terms: ['malware'],
        },
      ],
      tests: [{ input: { 'repository.name': 'analyse-malware' }, expected: 'match' }],
    },
  ],
};

const outcomeLabel: Record<RuleOutcome, string> = {
  match: 'Correspondance',
  no_match: 'Aucune correspondance',
  not_evaluated: 'Non évalué',
};

type Editing = { listId: string | null; text: string };

/** Fields of the sample: an empty field counts as not collected. */
type Sample = { name: string; description: string; topics: string };

function sampleInput(sample: Sample): Record<string, unknown> {
  const input: Record<string, unknown> = {};
  if (sample.name.trim()) input['repository.name'] = sample.name;
  if (sample.description.trim()) input['repository.description'] = sample.description;
  if (sample.topics.trim())
    input['repository.topics'] = sample.topics
      .split(',')
      .map((topic) => topic.trim())
      .filter(Boolean);
  return input;
}

function parse(text: string): Record<string, unknown> {
  try {
    return JSON.parse(text) as Record<string, unknown>;
  } catch (failure) {
    throw new Error('JSON invalide : ' + (failure as Error).message, { cause: failure });
  }
}

/** Versioned keyword lists of an investigation: publish, try on a sample, export. */
export function KeywordsTab({ investigationId, canWrite, action }: Props) {
  const { busy, run, setError } = action;
  const [lists, setLists] = useState<KeywordListSummary[]>([]);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [sample, setSample] = useState<Sample>({ name: '', description: '', topics: '' });
  const [preview, setPreview] = useState<ListPreview | null>(null);
  const [published, setPublished] = useState('');

  const reload = useCallback(
    () => api<KeywordListSummary[]>(`/investigations/${investigationId}/keyword-lists`).then(setLists),
    [investigationId],
  );
  useEffect(() => {
    reload().catch((failure: Error) => setError(failure.message));
  }, [reload, setError]);

  const [runs, setRuns] = useState<RuleRun[]>([]);
  const reloadRuns = useCallback(
    () => api<RuleRun[]>(`/investigations/${investigationId}/rule-runs`).then(setRuns),
    [investigationId],
  );
  const pending = runs.some(
    (evaluation) => evaluation.status === 'queued' || evaluation.status === 'running',
  );
  useEffect(() => {
    reloadRuns().catch((failure: Error) => setError(failure.message));
    if (!pending) return;
    // A queued or running evaluation is followed until it ends.
    const timer = setInterval(() => void reloadRuns().catch(() => undefined), 3000);
    return () => clearInterval(timer);
  }, [reloadRuns, pending, setError]);
  const evaluate = (list: KeywordListSummary) =>
    run(async () => {
      await api(`/keyword-lists/${list.id}/versions/${list.version}/runs`, 'POST', {});
      await reloadRuns();
    });
  const cancel = (evaluation: RuleRun) =>
    run(async () => {
      await api(`/rule-runs/${evaluation.id}/cancel`, 'POST', {});
      await reloadRuns();
    });

  const latest = (list: KeywordListSummary) =>
    api<KeywordVersion>(`/keyword-lists/${list.id}/versions/${list.version}`);
  const edit = (list: KeywordListSummary) =>
    run(async () => {
      const draft = { ...(await latest(list)).definition };
      // Attributed by the next publication.
      delete draft.version;
      delete draft.status;
      setEditing({ listId: list.id, text: JSON.stringify(draft, null, 2) });
      setPreview(null);
    });
  const exportList = (list: KeywordListSummary) =>
    run(async () => {
      const { definition } = await latest(list);
      const blob = new Blob([JSON.stringify(definition, null, 2) + '\n'], { type: 'application/json' });
      const link = document.createElement('a');
      link.href = URL.createObjectURL(blob);
      link.download = `${list.name.replace(/[^\p{L}\p{N}_-]+/gu, '-')}-v${list.version}.json`;
      link.click();
      URL.revokeObjectURL(link.href);
    });
  const tryList = () =>
    editing &&
    run(async () => {
      setPreview(
        await api<ListPreview>('/keyword-lists/preview', 'POST', {
          definition: parse(editing.text),
          input: sampleInput(sample),
        }),
      );
    });
  const publish = () =>
    editing &&
    run(async () => {
      const definition = parse(editing.text);
      const result = await api<{ version: number }>(
        editing.listId
          ? `/keyword-lists/${editing.listId}/versions`
          : `/investigations/${investigationId}/keyword-lists`,
        'POST',
        { definition },
      );
      setPublished(`Version ${result.version} publiée.`);
      setEditing(null);
      setPreview(null);
      await reload();
    });

  return (
    <section className="panel keywords">
      <h2>Listes de termes</h2>
      <p>
        Une version publiée ne change plus ; ses tests intégrés doivent réussir. Une correspondance signale un
        sujet à examiner, jamais une qualification du dépôt ou de son auteur.
      </p>
      {published && <p className="notice">{published}</p>}
      {canWrite && !editing && (
        <button
          className="secondary"
          onClick={() => {
            setEditing({ listId: null, text: JSON.stringify(TEMPLATE, null, 2) });
            setPublished('');
          }}
        >
          <Plus size={15} /> Nouvelle liste
        </button>
      )}
      {lists.length === 0 && !editing && <p>Aucune liste publiée dans ce dossier.</p>}
      {lists.length > 0 && (
        <table>
          <thead>
            <tr>
              <th>Liste</th>
              <th>Version</th>
              <th>Règles</th>
              <th>Publication</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {lists.map((list) => (
              <tr key={list.id}>
                <td>{list.name}</td>
                <td>v{list.version}</td>
                <td>{list.rules}</td>
                <td>
                  {list.published_by} · {formatDate(list.published_at)}
                </td>
                <td className="row-actions">
                  {canWrite && (
                    <button className="text-button" disabled={busy} onClick={() => void edit(list)}>
                      <Pencil size={13} /> Nouvelle version
                    </button>
                  )}
                  <button className="text-button" disabled={busy} onClick={() => void exportList(list)}>
                    <Download size={13} /> Exporter
                  </button>
                  {canWrite && (
                    <button className="text-button" disabled={busy} onClick={() => void evaluate(list)}>
                      <ListChecks size={13} /> Évaluer sur le dossier
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {runs.length > 0 && (
        <>
          <h3>Évaluations</h3>
          <p>
            Chaque évaluation lit les observations du dossier enregistrées à son démarrage ; ses
            correspondances alimentent l’onglet « Triage ».
          </p>
          <table aria-label="Évaluations">
            <thead>
              <tr>
                <th>Liste</th>
                <th>État</th>
                <th>Entités</th>
                <th>Correspondances</th>
                <th>Non évalués</th>
                <th>Demandée</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {runs.map((evaluation) => (
                <tr key={evaluation.id}>
                  <td>
                    {evaluation.list_name} v{evaluation.list_version}
                  </td>
                  <td>
                    <span className={'badge ' + evaluation.status}>{evaluation.status}</span>
                    {evaluation.error && <small> {evaluation.error}</small>}
                  </td>
                  <td>{evaluation.counts.entities ?? 0}</td>
                  <td>{evaluation.counts.match ?? 0}</td>
                  <td>{evaluation.counts.not_evaluated ?? 0}</td>
                  <td>{formatDate(evaluation.requested_at)}</td>
                  <td>
                    {canWrite && (evaluation.status === 'queued' || evaluation.status === 'running') && (
                      <button className="text-button" disabled={busy} onClick={() => void cancel(evaluation)}>
                        Annuler
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
      {editing && (
        <div className="keyword-editor">
          <label>
            {editing.listId ? 'Nouvelle version (JSON)' : 'Nouvelle liste (JSON)'}
            <textarea
              aria-label="Définition de la liste"
              spellCheck={false}
              value={editing.text}
              onChange={(event) => setEditing({ ...editing, text: event.target.value })}
            />
          </label>
          <fieldset>
            <legend>Exemple à essayer — un champ vide compte comme non collecté</legend>
            <label>
              Nom du dépôt
              <input
                value={sample.name}
                onChange={(event) => setSample({ ...sample, name: event.target.value })}
              />
            </label>
            <label>
              Description
              <input
                value={sample.description}
                onChange={(event) => setSample({ ...sample, description: event.target.value })}
              />
            </label>
            <label>
              Topics (séparés par des virgules)
              <input
                value={sample.topics}
                onChange={(event) => setSample({ ...sample, topics: event.target.value })}
              />
            </label>
          </fieldset>
          <div className="row-actions">
            <button className="secondary" disabled={busy} onClick={() => void tryList()}>
              <FlaskConical size={15} /> Essayer
            </button>
            <button disabled={busy} onClick={() => void publish()}>
              <Send size={15} /> Publier
            </button>
            <button className="text-button" onClick={() => setEditing(null)}>
              Abandonner
            </button>
          </div>
          {preview && (
            <div className="keyword-preview" aria-label="Résultat de l’essai">
              {preview.failing_tests.map((failure) => (
                <p className="notice" key={`${failure.rule}-${failure.test}`}>
                  Test {failure.test + 1} de « {failure.rule} » : {outcomeLabel[failure.expected]} attendu,{' '}
                  {outcomeLabel[failure.actual]} obtenu.
                </p>
              ))}
              {preview.results.map((result) => (
                <article key={result.rule}>
                  <h4>
                    {result.rule}{' '}
                    <span className={'badge ' + result.outcome}>{outcomeLabel[result.outcome]}</span>
                  </h4>
                  {result.hits.map((hit, index) => (
                    <p key={index}>
                      « {hit.term} » dans {hit.field} : <q>{hit.excerpt}</q>
                    </p>
                  ))}
                  {result.not_evaluated.length > 0 && <p>Non collecté : {result.not_evaluated.join(', ')}</p>}
                </article>
              ))}
            </div>
          )}
        </div>
      )}
    </section>
  );
}
