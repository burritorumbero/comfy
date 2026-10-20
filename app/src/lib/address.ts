/**
 * The server address as the app stores it: `scheme://host[:port]`, nothing after.
 *
 * Without a scheme, names default to https (what `tailscale serve` gives the Mac), and
 * bare local or Tailscale IP addresses to http, since certificates are issued for names.
 * Returns null when the text can't be an address.
 */
export function normalizeAddress(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed) ? trimmed : `${defaultScheme(trimmed)}://${trimmed}`;
  const match = /^(https?):\/\/([^/?#\s@]+)(?:[/?#]\S*)?$/i.exec(withScheme);
  if (!match) return null;
  const hostPort = /^(\[[0-9a-f:.]+\]|[a-z0-9-]+(?:\.[a-z0-9-]+)*)\.?(?::(\d{1,5}))?$/i.exec(match[2]);
  if (!hostPort) return null;
  const port = hostPort[2];
  if (port !== undefined && (Number(port) < 1 || Number(port) > 65535)) return null;
  return `${match[1].toLowerCase()}://${hostPort[1].toLowerCase()}${port ? `:${port}` : ''}`;
}

function defaultScheme(raw: string): 'http' | 'https' {
  const host = raw.split(/[/:?#]/)[0].toLowerCase();
  if (host === 'localhost') return 'http';
  const octets = host.split('.').map(Number);
  if (octets.length !== 4 || octets.some((o) => !Number.isInteger(o) || o < 0 || o > 255)) return 'https';
  const [a, b] = octets;
  const local =
    a === 127 || a === 10 || (a === 192 && b === 168) || (a === 172 && b >= 16 && b <= 31) || (a === 100 && b >= 64 && b <= 127);
  return local ? 'http' : 'https';
}

/** `my-mac.tail1a2b3c.ts.net` from `https://my-mac.tail1a2b3c.ts.net`, for messages. */
export function hostOf(address: string): string {
  return address.replace(/^https?:\/\//, '');
}
