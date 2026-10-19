import type { Job } from '../api';
import { appendPage, mergeFirstPage, statusText, upsert } from '../jobs';

const job = (id: string, created: number, extra: Partial<Job> = {}): Job => ({ id, prompt: id, created, state: 'done', ...extra });
const ids = (jobs: Job[]) => jobs.map((j) => j.id);

describe('statusText', () => {
  const now = 1_000_000;
  it('says where a job is', () => {
    expect(statusText(job('a', now, { state: 'queued', ahead: 2 }), now)).toBe('Queued · 2 ahead');
    expect(statusText(job('a', now, { state: 'queued' }), now)).toBe('Starting…');
    expect(statusText(job('a', now, { state: 'running', progress: { value: 23, max: 50 } }), now + 41_000, now)).toBe('Step 23 of 50 · 0:41');
  });

  it('says how it ended', () => {
    expect(statusText(job('a', now, { seconds: 41.5, seed: 7 }), now + 30_000)).toBe('Done in 41.5 s · just now · seed 7');
    expect(statusText(job('a', now, { state: 'error', error: 'UNETLoader failed: Out of memory' }), now)).toBe('UNETLoader failed: Out of memory');
    expect(statusText(job('a', now, { state: 'cancelled' }), now)).toBe('Cancelled');
  });
});

describe('history merging', () => {
  it('adds new jobs, updates known ones, and drops ones removed elsewhere', () => {
    const shown = [job('c', 30, { state: 'running' }), job('b', 20), job('a', 10)];
    const merged = mergeFirstPage(shown, { jobs: [job('d', 40), job('c', 30)], more: false });
    expect(ids(merged)).toEqual(['d', 'c']);
    expect(merged[1].state).toBe('done');
  });

  it('keeps older pages that the newest page does not cover', () => {
    const shown = [job('c', 30), job('b', 20), job('a', 10)];
    expect(ids(mergeFirstPage(shown, { jobs: [job('d', 40), job('c', 30)], more: true }))).toEqual(['d', 'c', 'b', 'a']);
  });

  it('appends older pages and upserts single jobs in order', () => {
    expect(ids(appendPage([job('c', 30)], { jobs: [job('c', 30), job('b', 20)], more: false }))).toEqual(['c', 'b']);
    expect(ids(upsert([job('b', 20)], job('c', 30)))).toEqual(['c', 'b']);
  });
});
