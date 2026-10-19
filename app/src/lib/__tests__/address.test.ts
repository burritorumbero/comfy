import { normalizeAddress } from '../address';

describe('normalizeAddress', () => {
  it('keeps what tailscale serve prints', () => {
    expect(normalizeAddress('https://my-mac.tail1a2b3c.ts.net')).toBe('https://my-mac.tail1a2b3c.ts.net');
  });

  it('defaults names to https and drops anything after the host', () => {
    expect(normalizeAddress('  My-Mac.tail1a2b3c.ts.net/  ')).toBe('https://my-mac.tail1a2b3c.ts.net');
    expect(normalizeAddress('my-mac.tail1a2b3c.ts.net:8443/api/info?x=1')).toBe('https://my-mac.tail1a2b3c.ts.net:8443');
  });

  it('defaults local and Tailscale IP addresses to http', () => {
    expect(normalizeAddress('192.168.1.20:8189')).toBe('http://192.168.1.20:8189');
    expect(normalizeAddress('100.101.102.103:8189')).toBe('http://100.101.102.103:8189');
    expect(normalizeAddress('localhost:8189')).toBe('http://localhost:8189');
    expect(normalizeAddress('8.8.8.8')).toBe('https://8.8.8.8');
  });

  it('refuses what cannot be an address', () => {
    for (const bad of ['', '   ', 'ftp://host', 'https://', 'not an address', 'https://host:99999', 'https://user@host']) {
      expect(normalizeAddress(bad)).toBeNull();
    }
  });
});
