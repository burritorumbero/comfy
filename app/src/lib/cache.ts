import { File, Paths } from 'expo-file-system';
import { Platform } from 'react-native';

import type { Job } from './api';

// The newest page of history, kept on the phone so the list shows instantly, and offline.
// It belongs to one server: a cache for another address is ignored.
type Cached = { address: string; jobs: Job[] };

function file(): File | null {
  return Platform.OS === 'web' ? null : new File(Paths.document, 'history.json');
}

export async function loadHistory(address: string): Promise<Job[]> {
  try {
    const f = file();
    if (!f?.exists) return [];
    const cached = JSON.parse(await f.text()) as Cached;
    return cached.address === address && Array.isArray(cached.jobs) ? cached.jobs : [];
  } catch {
    return [];
  }
}

export function saveHistory(address: string, jobs: Job[]): void {
  try {
    file()?.write(JSON.stringify({ address, jobs } satisfies Cached));
  } catch {
    // A missing cache only costs a slower first paint.
  }
}

export function clearHistory(): void {
  try {
    const f = file();
    if (f?.exists) f.delete();
  } catch {
    // Nothing to clear.
  }
}
