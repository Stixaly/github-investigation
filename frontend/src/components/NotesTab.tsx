import { api } from '../api';
import { formatDate } from '../format';
import type { Action } from '../hooks/useAction';
import type { Note } from '../types';

type Props = {
  investigationId: string;
  notes: Note[];
  canWrite: boolean;
  action: Action;
  onNotes: (notes: Note[]) => void;
};

export function NotesTab({ investigationId, notes, canWrite, action, onNotes }: Props) {
  const { busy, run } = action;
  return (
    <section className="panel">
      <h2>Notes d’analyse</h2>
      <p>Les hypothèses restent séparées des observations du graphe.</p>
      {canWrite && (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            const form = event.currentTarget;
            const body = new FormData(form).get('body');
            void run(async () => {
              await api(`/investigations/${investigationId}/notes`, 'POST', { body });
              onNotes(await api<Note[]>(`/investigations/${investigationId}/notes`));
              form.reset();
            });
          }}
        >
          <label>
            Observation ou hypothèse
            <textarea
              name="body"
              required
              maxLength={10000}
              placeholder="Décrivez votre hypothèse et ce qui permet de l’étayer…"
            />
          </label>
          <button disabled={busy}>Enregistrer la note</button>
        </form>
      )}
      {notes.map((note) => (
        <article className="note" key={note.id}>
          <small>
            {note.login} · {formatDate(note.created_at)}
          </small>
          <p>{note.body}</p>
        </article>
      ))}
    </section>
  );
}
