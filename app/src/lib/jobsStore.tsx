import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';

import { ApiError, api, type GenerateRequest, type Job, type ServerInfo } from './api';
import { useAppActive } from './appActive';
import { loadHistory, saveHistory } from './cache';
import { useConnection } from './connection';
import { appendPage, isActive, mergeFirstPage, upsert } from './jobs';

type JobsState = {
  jobs: Job[];
  more: boolean;
  info: ServerInfo | null;
  /** Why the server can't be reached right now, if it can't. */
  problem: string | null;
  /** When each job was first seen running, for its clock. */
  runningSince: Map<string, number>;
  /** Width / height of the last image shown, to size placeholders. */
  aspectRatio: number;
  setAspectRatio: (ratio: number) => void;
  refresh: () => Promise<void>;
  loadOlder: () => Promise<void>;
  generate: (request: GenerateRequest) => Promise<void>;
  cancel: (id: string) => Promise<void>;
  remove: (id: string) => Promise<void>;
};

const Context = createContext<JobsState | null>(null);

/**
 * The history, shared with every other device through the server. Active jobs are checked
 * every second and the newest page every ten, only while the app is in the foreground.
 */
export function JobsProvider({ children }: { children: ReactNode }) {
  const { connection, rejected, markRejected } = useConnection();
  const appActive = useAppActive();
  const address = connection?.address;
  const [jobs, setJobs] = useState<Job[]>([]);
  const [more, setMore] = useState(false);
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [aspectRatio, setAspectRatio] = useState(1);
  const [runningSince] = useState(() => new Map<string, number>());

  // A different server (or none): nothing from the previous one may show.
  const [shownFor, setShownFor] = useState(address);
  if (shownFor !== address) {
    setShownFor(address);
    setJobs([]);
    setInfo(null);
    setMore(false);
    setProblem(null);
  }

  // The latest list, for timers and callbacks that run outside rendering.
  const latest = useRef(jobs);
  const loadedPastFirstPage = useRef(false);
  const busy = useRef(false);
  useEffect(() => {
    latest.current = jobs;
    for (const job of jobs) if (job.state === 'running' && !runningSince.has(job.id)) runningSince.set(job.id, Date.now());
  }, [jobs, runningSince]);

  useEffect(() => {
    loadedPastFirstPage.current = false;
    if (!address) return;
    let current = true;
    loadHistory(address).then((cached) => current && setJobs((shown) => (shown.length ? shown : cached)));
    return () => {
      current = false;
    };
  }, [address]);

  const fail = useCallback(
    (error: unknown) => {
      if (error instanceof ApiError && error.status === 401) markRejected();
      else setProblem(error instanceof Error ? error.message : String(error));
    },
    [markRejected],
  );

  const refresh = useCallback(async () => {
    if (!connection) return;
    try {
      const [page, serverInfo] = await Promise.all([api.jobs(connection), api.info(connection)]);
      setJobs((shown) => mergeFirstPage(shown, page));
      if (!loadedPastFirstPage.current) setMore(page.more);
      setInfo(serverInfo);
      setProblem(null);
      saveHistory(connection.address, page.jobs);
    } catch (error) {
      fail(error);
    }
  }, [connection, fail]);

  const pollActive = useCallback(async () => {
    if (!connection) return;
    for (const job of latest.current.filter(isActive)) {
      try {
        const fresh = await api.job(connection, job.id);
        setJobs((shown) => upsert(shown, fresh));
      } catch (error) {
        if (error instanceof ApiError && error.status === 404) setJobs((shown) => shown.filter((j) => j.id !== job.id));
        else return fail(error);
      }
    }
  }, [connection, fail]);

  useEffect(() => {
    if (!connection || !appActive || rejected) return;
    let tick = 0;
    const run = async () => {
      if (busy.current) return;
      busy.current = true;
      try {
        if (tick % 10 === 0) await refresh();
        else if (latest.current.some(isActive)) await pollActive();
      } finally {
        busy.current = false;
        tick += 1;
      }
    };
    // The first run refreshes everything; then active jobs every second, the list every ten.
    const first = setTimeout(run, 0);
    const timer = setInterval(run, 1000);
    return () => {
      clearTimeout(first);
      clearInterval(timer);
    };
  }, [connection, appActive, rejected, refresh, pollActive]);

  const loadOlder = useCallback(async () => {
    const oldest = latest.current[latest.current.length - 1];
    if (!connection || !oldest || !more) return;
    try {
      const page = await api.jobs(connection, oldest.id);
      loadedPastFirstPage.current = true;
      setJobs((shown) => appendPage(shown, page));
      setMore(page.more);
    } catch (error) {
      fail(error);
    }
  }, [connection, more, fail]);

  // Actions throw so the screen can say what went wrong where it happened.
  const generate = useCallback(
    async (request: GenerateRequest) => {
      if (!connection) return;
      try {
        const job = await api.generate(connection, request);
        setJobs((shown) => upsert(shown, job));
      } catch (error) {
        if (error instanceof ApiError && error.status === 401) markRejected();
        throw error;
      }
    },
    [connection, markRejected],
  );

  const cancel = useCallback(
    async (id: string) => {
      if (!connection) return;
      const job = await api.cancel(connection, id);
      setJobs((shown) => upsert(shown, job));
    },
    [connection],
  );

  const remove = useCallback(
    async (id: string) => {
      if (!connection) return;
      try {
        await api.remove(connection, id);
      } catch (error) {
        if (!(error instanceof ApiError && error.status === 404)) throw error;
      }
      setJobs((shown) => shown.filter((j) => j.id !== id));
    },
    [connection],
  );

  const value = useMemo(
    () => ({ jobs, more, info, problem, runningSince, aspectRatio, setAspectRatio, refresh, loadOlder, generate, cancel, remove }),
    [jobs, more, info, problem, runningSince, aspectRatio, refresh, loadOlder, generate, cancel, remove],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useJobs(): JobsState {
  const value = useContext(Context);
  if (!value) throw new Error('useJobs outside JobsProvider');
  return value;
}
