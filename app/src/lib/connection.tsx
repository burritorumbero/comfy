import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';

import type { Connection } from './api';
import { clearHistory } from './cache';
import { clearImages } from './save';
import { deleteSecret, readJson, writeSecret } from './storage';

const KEY = 'connection';

type ConnectionState = {
  /** False until the Keychain has been read. */
  ready: boolean;
  connection: Connection | null;
  /** The server stopped accepting the saved password (it was changed on the Mac). */
  rejected: boolean;
  save: (connection: Connection) => Promise<void>;
  forget: () => Promise<void>;
  markRejected: () => void;
};

const Context = createContext<ConnectionState | null>(null);

/** The one server this app talks to: its address and password, set once on the Connect screen. */
export function ConnectionProvider({ children }: { children: ReactNode }) {
  const [ready, setReady] = useState(false);
  const [connection, setConnection] = useState<Connection | null>(null);
  const [rejected, setRejected] = useState(false);

  useEffect(() => {
    readJson<Connection>(KEY).then((saved) => {
      if (saved?.address && typeof saved.password === 'string') setConnection(saved);
      setReady(true);
    });
  }, []);

  const save = useCallback(async (next: Connection) => {
    await writeSecret(KEY, JSON.stringify(next));
    setConnection((previous) => {
      // Another server's history and images must never show under this one.
      if (previous && previous.address !== next.address) {
        clearHistory();
        void clearImages();
      }
      return next;
    });
    setRejected(false);
  }, []);

  const forget = useCallback(async () => {
    await deleteSecret(KEY);
    clearHistory();
    await clearImages();
    setConnection(null);
    setRejected(false);
  }, []);

  const markRejected = useCallback(() => setRejected(true), []);

  const value = useMemo(
    () => ({ ready, connection, rejected, save, forget, markRejected }),
    [ready, connection, rejected, save, forget, markRejected],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useConnection(): ConnectionState {
  const value = useContext(Context);
  if (!value) throw new Error('useConnection outside ConnectionProvider');
  return value;
}
