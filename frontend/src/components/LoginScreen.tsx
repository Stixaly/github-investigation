import { ArrowUpRight, CircleDot, GitBranch, LockKeyhole } from 'lucide-react';

type Props = {
  busy: boolean;
  error: string;
  onLogin: (login: string, password: string) => void;
};

export function LoginScreen({ busy, error, onLogin }: Props) {
  return (
    <main className="login">
      <div className="login-story">
        <div className="brand">
          <CircleDot /> OBSERVATORY
        </div>
        <p className="eyebrow">GITHUB INVESTIGATION</p>
        <h1>
          Les relations
          <br />
          ont une histoire.
        </h1>
        <p>
          Explorez les comptes, les dépôts et les observations qui les relient. Construisez vos investigations
          sur des faits traçables.
        </p>
        <div className="login-art">
          <GitBranch size={120} strokeWidth={0.7} />
          <span>COMPTES · DÉPÔTS · PREUVES</span>
        </div>
      </div>
      <section className="login-form">
        <LockKeyhole size={26} />
        <h2>Votre espace d’investigation</h2>
        <p>Connectez-vous avec votre compte analyste.</p>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            const form = new FormData(event.currentTarget);
            onLogin(String(form.get('login') ?? ''), String(form.get('password') ?? ''));
          }}
        >
          <label>
            Identifiant
            <input name="login" required autoComplete="username" autoFocus />
          </label>
          <label>
            Mot de passe
            <input name="password" type="password" required autoComplete="current-password" />
          </label>
          <button disabled={busy}>
            Ouvrir mon espace <ArrowUpRight size={17} />
          </button>
        </form>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
        <small>Accès nominatif · Investigations à accès contrôlé</small>
      </section>
    </main>
  );
}
