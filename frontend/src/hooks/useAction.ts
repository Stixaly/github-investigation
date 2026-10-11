import { useCallback, useState } from 'react';

/** Runs user-triggered async actions one at a time and keeps the last error for display. */
export function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const run = useCallback(async (action: () => Promise<void>) => {
    setError('');
    setBusy(true);
    try {
      await action();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setBusy(false);
    }
  }, []);
  return { busy, error, setError, run };
}

export type Action = ReturnType<typeof useAction>;
