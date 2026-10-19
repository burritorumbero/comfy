import { useEffect, useState } from 'react';

/** The current time, refreshed every second while `fast` (a running clock) and every minute otherwise ("5 min ago"). */
export function useNow(fast: boolean): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), fast ? 1000 : 60_000);
    return () => clearInterval(timer);
  }, [fast]);
  return now;
}
