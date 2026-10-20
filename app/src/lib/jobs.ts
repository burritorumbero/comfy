import type { Job, JobPage } from './api';

export const isActive = (job: Job) => job.state === 'queued' || job.state === 'running';

export function clock(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

export function ago(ms: number, now: number): string {
  if (!ms) return '';
  const minutes = Math.floor((now - ms) / 60000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes} min ago`;
  if (minutes < 48 * 60) return `${Math.round(minutes / 60)} h ago`;
  return new Date(ms).toLocaleDateString();
}

/** One line under a job: where it is, or how it ended. */
export function statusText(job: Job, now: number, runningSince?: number): string {
  switch (job.state) {
    case 'queued':
      return job.ahead ? `Queued · ${job.ahead} ahead` : 'Starting…';
    case 'running': {
      const p = job.progress;
      const steps = p && p.max > 1 ? `Step ${p.value} of ${p.max}` : job.kind === 'edit' ? 'Editing' : 'Generating';
      const since = runningSince ?? job.created;
      return since ? `${steps} · ${clock(now - since)}` : steps;
    }
    case 'done': {
      const parts = [job.seconds != null ? `Done in ${job.seconds} s` : 'Done', ago(job.created, now)];
      if (job.seed != null) parts.push(`seed ${job.seed}`);
      return parts.filter(Boolean).join(' · ');
    }
    case 'cancelled':
      return 'Cancelled';
    default:
      return job.error ?? 'Failed';
  }
}

const newestFirst = (a: Job, b: Job) => b.created - a.created;

/** Add or replace one job, keeping the list newest first. */
export function upsert(jobs: Job[], job: Job): Job[] {
  const index = jobs.findIndex((j) => j.id === job.id);
  if (index < 0) return [...jobs, job].sort(newestFirst);
  const next = jobs.slice();
  next[index] = job;
  return next;
}

/**
 * Merge the server's newest page into what is shown: add new jobs, update known ones,
 * and drop the ones another device removed from that window.
 */
export function mergeFirstPage(jobs: Job[], page: JobPage): Job[] {
  const fetched = new Set(page.jobs.map((j) => j.id));
  const cutoff = page.more && page.jobs.length ? Math.min(...page.jobs.map((j) => j.created)) : -Infinity;
  const kept = jobs.filter((j) => !fetched.has(j.id) && j.created < cutoff);
  return [...page.jobs, ...kept].sort(newestFirst);
}

/** Append an older page. */
export function appendPage(jobs: Job[], page: JobPage): Job[] {
  const known = new Set(jobs.map((j) => j.id));
  return [...jobs, ...page.jobs.filter((j) => !known.has(j.id))].sort(newestFirst);
}
