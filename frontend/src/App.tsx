import { useCallback, useEffect, useState } from 'react';
import { FolderSearch, Layers, Plus, ShieldCheck, X } from 'lucide-react';
import { api, setCsrfToken } from './api';
import { InvestigationView } from './components/InvestigationView';
import { LoginScreen } from './components/LoginScreen';
import { NewInvestigationModal } from './components/NewInvestigationModal';
import { Sidebar } from './components/Sidebar';
import { useAction } from './hooks/useAction';
import type { Investigation, User } from './types';

export default function App() {
  const action = useAction();
  const { busy, error, setError, run } = action;
  const [user, setUser] = useState<User | null>(null);
  const [authReady, setAuthReady] = useState(false);
  const [investigations, setInvestigations] = useState<Investigation[]>([]);
  const [activeId, setActiveId] = useState('');
  const [modal, setModal] = useState(false);

  const signIn = (session: User) => {
    setCsrfToken(session.csrf);
    setUser(session);
  };
  const refresh = useCallback(async () => {
    const list = await api<Investigation[]>('/investigations');
    setInvestigations(list);
    setActiveId((id) => (list.some((item) => item.id === id) ? id : (list[0]?.id ?? '')));
  }, []);

  useEffect(() => {
    api<User>('/me')
      .then((session) => {
        setCsrfToken(session.csrf);
        setUser(session);
      })
      .catch(() => undefined)
      .finally(() => setAuthReady(true));
  }, []);
  useEffect(() => {
    if (user) void run(refresh);
  }, [user, run, refresh]);

  if (!authReady) {
    return (
      <main className="login">
        <p>Ouverture de l’espace…</p>
      </main>
    );
  }
  if (!user) {
    return (
      <LoginScreen
        busy={busy}
        error={error}
        onLogin={(login, password) =>
          void run(async () => signIn(await api<User>('/login', 'POST', { login, password })))
        }
      />
    );
  }
  const active = investigations.find((item) => item.id === activeId);
  return (
    <div className="shell">
      <Sidebar
        user={user}
        investigations={investigations}
        activeId={activeId}
        onSelect={setActiveId}
        onCreate={() => setModal(true)}
        onLogout={() =>
          void run(async () => {
            await api('/logout', 'POST');
            setCsrfToken('');
            setUser(null);
            setInvestigations([]);
            setActiveId('');
          })
        }
      />
      <main className="workspace">
        <header className="topbar">
          <span>
            <Layers size={15} /> INVESTIGATIONS <span className="slash">/</span>{' '}
            {active?.title || 'Nouvel espace'}
          </span>
          <span className="private">
            <ShieldCheck size={14} /> Accès contrôlé
          </span>
        </header>
        {error && (
          <div className="error banner" role="alert">
            {error}
            <button className="icon" aria-label="Fermer l’erreur" onClick={() => setError('')}>
              <X size={16} />
            </button>
          </div>
        )}
        {active ? (
          <InvestigationView
            key={active.id}
            investigation={active}
            action={action}
            onInvestigationsChanged={refresh}
            onOpenInvestigation={setActiveId}
          />
        ) : (
          <section className="empty welcome">
            <FolderSearch size={44} />
            <p className="eyebrow">VOTRE PREMIER POINT DE DÉPART</p>
            <h1>
              Une investigation commence
              <br />
              par une question.
            </h1>
            <p>Créez un dossier, puis explorez un compte ou un dépôt public GitHub.</p>
            <button onClick={() => setModal(true)}>
              <Plus size={16} />
              Créer une investigation
            </button>
          </section>
        )}
        <footer className="app-footer">
          OBSERVATORY <span>v0.2 · Socle d’investigation · Les données de démonstration sont fictives.</span>
        </footer>
      </main>
      {modal && (
        <NewInvestigationModal
          busy={busy}
          onClose={() => setModal(false)}
          onCreate={(title, description) =>
            void run(async () => {
              const created = await api<Investigation>('/investigations', 'POST', { title, description });
              await refresh();
              setActiveId(created.id);
              setModal(false);
            })
          }
        />
      )}
    </div>
  );
}
