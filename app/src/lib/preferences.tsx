import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';

import { readJson, writeSecret } from './storage';

const KEY = 'preferences';

type Preferences = { hideInSwitcher: boolean };
const defaults: Preferences = { hideInSwitcher: true };

type PreferencesState = Preferences & { setHideInSwitcher: (on: boolean) => void };

const Context = createContext<PreferencesState | null>(null);

/** Settings that apply on this iPhone only. */
export function PreferencesProvider({ children }: { children: ReactNode }) {
  const [preferences, setPreferences] = useState<Preferences>(defaults);

  useEffect(() => {
    readJson<Partial<Preferences>>(KEY).then((saved) => saved && setPreferences({ ...defaults, ...saved }));
  }, []);

  const setHideInSwitcher = useCallback((on: boolean) => {
    setPreferences((current) => {
      const next = { ...current, hideInSwitcher: on };
      void writeSecret(KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  const value = useMemo(() => ({ ...preferences, setHideInSwitcher }), [preferences, setHideInSwitcher]);
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function usePreferences(): PreferencesState {
  const value = useContext(Context);
  if (!value) throw new Error('usePreferences outside PreferencesProvider');
  return value;
}
