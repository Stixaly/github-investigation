import { api } from '../api';
import { initials, roleLabel } from '../format';
import type { Action } from '../hooks/useAction';
import type { Member } from '../types';

type Props = {
  investigationId: string;
  members: Member[];
  isOwner: boolean;
  action: Action;
  onMembers: (members: Member[]) => void;
};

export function AccessTab({ investigationId, members, isOwner, action, onMembers }: Props) {
  const { busy, run } = action;
  const path = `/investigations/${investigationId}/members`;
  return (
    <section className="panel">
      <h2>Membres de l’investigation</h2>
      <p>
        Les comptes sont créés par l’administrateur via la CLI. Ajoutez ensuite les membres autorisés à ce
        dossier.
      </p>
      {members.map((member) => (
        <div className="member" key={member.login}>
          <span className="avatar">{initials(member.login)}</span>
          <strong>{member.login}</strong>
          <span>{roleLabel[member.role]}</span>
          {isOwner && member.role !== 'owner' && (
            <button
              className="secondary"
              onClick={() =>
                void run(async () => {
                  await api(`${path}/${encodeURIComponent(member.login)}`, 'DELETE');
                  onMembers(await api<Member[]>(path));
                })
              }
            >
              Retirer l’accès
            </button>
          )}
        </div>
      ))}
      {isOwner && (
        <form
          className="member-form"
          onSubmit={(event) => {
            event.preventDefault();
            const form = event.currentTarget;
            const fields = new FormData(form);
            void run(async () => {
              await api(path, 'PUT', { login: fields.get('login'), role: fields.get('role') });
              onMembers(await api<Member[]>(path));
              form.reset();
            });
          }}
        >
          <label>
            Identifiant
            <input name="login" required minLength={3} />
          </label>
          <label>
            Rôle
            <select name="role">
              <option value="analyst">Analyste</option>
              <option value="reader">Lecteur</option>
            </select>
          </label>
          <button disabled={busy}>Ajouter le membre</button>
        </form>
      )}
    </section>
  );
}
