import { ArrowUpRight, X } from 'lucide-react';

type Props = {
  busy: boolean;
  onClose: () => void;
  onCreate: (title: string, description: string) => void;
};

export function NewInvestigationModal({ busy, onClose, onCreate }: Props) {
  return (
    <div className="modal-overlay" role="presentation">
      <section className="modal" role="dialog" aria-modal="true" aria-labelledby="modal-title">
        <button className="icon modal-close" aria-label="Fermer" onClick={onClose}>
          <X />
        </button>
        <p className="eyebrow">NOUVEAU DOSSIER</p>
        <h2 id="modal-title">Créer une investigation</h2>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            const form = new FormData(event.currentTarget);
            onCreate(String(form.get('title') ?? ''), String(form.get('description') ?? ''));
          }}
        >
          <label>
            Titre
            <input name="title" required maxLength={160} autoFocus />
          </label>
          <label>
            Objectif
            <textarea
              name="description"
              maxLength={5000}
              placeholder="Quel réseau ou quelle hypothèse souhaitez-vous explorer ?"
            />
          </label>
          <p>Vous serez responsable de ce dossier. Vous pourrez y inviter d’autres analystes.</p>
          <button disabled={busy}>
            Créer le dossier <ArrowUpRight size={16} />
          </button>
        </form>
      </section>
    </div>
  );
}
