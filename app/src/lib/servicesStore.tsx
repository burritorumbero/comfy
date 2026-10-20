import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';

import { ApiError, api, type Mode, type Services } from './api';
import { useAppActive } from './appActive';
import { useConnection } from './connection';
import { confirmDestructive, showMessage } from './dialogs';
import { useJobs } from './jobsStore';
import { modeLabel, switchQuestion } from './services';

type ServicesState = {
  /** What the Mac runs, or null before the first answer or on a server without switching. */
  services: Services | null;
  refresh: () => Promise<void>;
  switchTo: (to: Mode, cancelJobs?: boolean) => Promise<void>;
};

const Context = createContext<ServicesState | null>(null);

/**
 * What the Mac's memory is set aside for (Generate, Edit or the LLM), checked every ten seconds while the app is in the
 * foreground, and every second while a switch runs.
 */
export function ServicesProvider({ children }: { children: ReactNode }) {
  const { connection, rejected, markRejected } = useConnection();
  const { info } = useJobs();
  const appActive = useAppActive();
  const supported = info?.services === true;
  const address = connection?.address;
  const [services, setServices] = useState<Services | null>(null);

  const [shownFor, setShownFor] = useState(address);
  if (shownFor !== address) {
    setShownFor(address);
    setServices(null);
  }

  const refresh = useCallback(async () => {
    if (!connection) return;
    try {
      setServices(await api.services(connection));
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) markRejected();
      // Otherwise keep the last answer: the history's banner already says the Mac is unreachable.
    }
  }, [connection, markRejected]);

  const switching = services?.switching != null;
  useEffect(() => {
    if (!connection || !appActive || rejected || !supported) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;
    const tick = async () => {
      await refresh();
      if (!stopped) timer = setTimeout(tick, switching ? 1000 : 10000);
    };
    void tick();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [connection, appActive, rejected, supported, switching, refresh]);

  const switchTo = useCallback(
    async (to: Mode, cancelJobs = false) => {
      if (!connection) return;
      setServices(await api.switchTo(connection, to, cancelJobs));
    },
    [connection],
  );

  const value = useMemo(() => ({ services: supported ? services : null, refresh, switchTo }), [supported, services, refresh, switchTo]);
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useServices(): ServicesState {
  const value = useContext(Context);
  if (!value) throw new Error('useServices outside ServicesProvider');
  return value;
}

/**
 * Switch after saying what stops, and ask again before cancelling image jobs. Resolves true once
 * the Mac has started the switch.
 */
export function useRequestSwitch(): (to: Mode) => Promise<boolean> {
  const { services, switchTo } = useServices();
  return useCallback(
    async (to: Mode) => {
      const question = switchQuestion(services, to);
      if (question && !(await confirmDestructive(question.title, question.message, 'Switch'))) return false;
      try {
        await switchTo(to);
        return true;
      } catch (error) {
        if (error instanceof ApiError && error.status === 409 && error.activeJobs) {
          const n = error.activeJobs;
          const cancel = await confirmDestructive(
            `Cancel ${n} image job${n === 1 ? '' : 's'}?`,
            `${n === 1 ? 'It is' : 'They are'} still queued or running on your Mac. Switching to ${to === 'llm' ? 'the LLM' : modeLabel[to]} stops ${n === 1 ? 'it' : 'them'}.`,
            'Cancel and switch',
          );
          if (!cancel) return false;
          try {
            await switchTo(to, true);
            return true;
          } catch (again) {
            showMessage("Couldn't switch", again instanceof Error ? again.message : String(again));
            return false;
          }
        }
        showMessage("Couldn't switch", error instanceof Error ? error.message : String(error));
        return false;
      }
    },
    [services, switchTo],
  );
}
