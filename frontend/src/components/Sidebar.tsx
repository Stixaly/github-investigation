import { useEffect, useState } from 'react';
import { CircleDot, FolderSearch, LogOut, Plus, Search } from 'lucide-react';
import { api } from '../api';
import { initials, kindLabel, roleLabel } from '../format';
import type { Found, Investigation, User } from '../types';

type Props = {
  user: User;
  investigations: Investigation[];
  activeId: string;
  onSelect: (id: string) => void;
  onCreate: () => void;
  onLogout: () => void;
};

/** Pause after the last keystroke before searching. */
const SEARCH_DELAY_MS = 250;

/** Accounts and repositories observed in the analyst's investigations, by name fragment. */
function useSearch(query: string) {
  const [found, setFound] = useState<Found[]>([]);
  const [searched, setSearched] = useState('');
  const text = query.trim();
  useEffect(() => {
    if (text.length < 2) return;
    let live = true;
    const timer = setTimeout(() => {
      api<Found[]>(`/search?q=${encodeURIComponent(text)}`)
        .catch(() => [])
        .then((rows) => {
          if (live) {
            setFound(rows);
            setSearched(text);
          }
        });
    }, SEARCH_DELAY_MS);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [text]);
  const current = text.length >= 2 && searched === text;
  return { found: current ? found : [], done: current };
}

export function Sidebar({ user, investigations, activeId, onSelect, onCreate, onLogout }: Props) {
  const [query, setQuery] = useState('');
  const { found, done } = useSearch(query);
  return (
    <aside className="sidebar">
      <div className="brand">
        <CircleDot /> OBSERVATORY
      </div>
      <p className="eyebrow">ESPACE D’ANALYSE</p>
      <label className="sidebar-search">
        <Search size={15} />
        <input
          type="search"
          aria-label="Rechercher dans tous les dossiers"
          placeholder="Compte ou dépôt…"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      </label>
      {done && (
        <div className="search-results" aria-label="Résultats de recherche">
          {found.length === 0 && <p className="search-empty">Aucune entité observée sous ce nom.</p>}
          {found.map((entity) => (
            <div className="search-result" key={entity.id}>
              {entity.label}
              <small>{kindLabel[entity.kind] ?? entity.kind}</small>
              {entity.investigations.map((investigation) => (
                <button
                  key={investigation.id}
                  onClick={() => {
                    onSelect(investigation.id);
                    setQuery('');
                  }}
                >
                  {investigation.title}
                </button>
              ))}
            </div>
          ))}
        </div>
      )}
      <div className="nav-heading">
        <span>Investigations</span>
        <button className="icon" aria-label="Nouvelle investigation" onClick={onCreate}>
          <Plus size={18} />
        </button>
      </div>
      <nav>
        {investigations.map((investigation) => (
          <button
            key={investigation.id}
            className={activeId === investigation.id ? 'nav-item active' : 'nav-item'}
            onClick={() => onSelect(investigation.id)}
          >
            <FolderSearch size={18} />
            <span>
              {investigation.title}
              <small>{roleLabel[investigation.role]}</small>
            </span>
          </button>
        ))}
      </nav>
      <div className="sidebar-bottom">
        <div className="status-line">
          <span className="dot" />
          Corpus GitHub public
        </div>
        <p>
          Chaque lien conserve son origine.
          <br />
          Chaque scan garde sa perspective.
        </p>
        <div className="profile">
          <span className="avatar">{initials(user.login)}</span>
          <span>
            {user.login}
            <small>Compte analyste</small>
          </span>
          <button className="icon" aria-label="Déconnexion" onClick={onLogout}>
            <LogOut size={17} />
          </button>
        </div>
      </div>
    </aside>
  );
}
