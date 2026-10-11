import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { LoginScreen } from './LoginScreen';

describe('LoginScreen', () => {
  it('submits the credentials', async () => {
    const onLogin = vi.fn();
    render(<LoginScreen busy={false} error="" onLogin={onLogin} />);
    await userEvent.type(screen.getByLabelText('Identifiant'), 'analyste');
    await userEvent.type(screen.getByLabelText('Mot de passe'), 'secret-passphrase');
    await userEvent.click(screen.getByRole('button', { name: 'Ouvrir mon espace' }));
    expect(onLogin).toHaveBeenCalledWith('analyste', 'secret-passphrase');
  });

  it('shows the refusal and blocks resubmission while busy', () => {
    render(<LoginScreen busy error="Identifiants incorrects" onLogin={vi.fn()} />);
    expect(screen.getByRole('alert')).toHaveTextContent('Identifiants incorrects');
    expect(screen.getByRole('button', { name: 'Ouvrir mon espace' })).toBeDisabled();
  });
});
