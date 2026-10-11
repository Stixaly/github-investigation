import { CircleDot, FolderSearch, LogOut, Plus } from 'lucide-react';
import { initials, roleLabel } from '../format';
import type { Investigation, User } from '../types';

type Props = {
  user: User;
  investigations: Investigation[];
  activeId: string;
  onSelect: (id: string) => void;
  onCreate: () => void;
  onLogout: () => void;
};

export function Sidebar({ user, investigations, activeId, onSelect, onCreate, onLogout }: Props) {
  return (
    <aside className="sidebar">
      <div className="brand">
        <CircleDot /> OBSERVATORY
      </div>
      <p className="eyebrow">ESPACE D’ANALYSE</p>
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
