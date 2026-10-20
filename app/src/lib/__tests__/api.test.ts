import { api, testConnection } from '../api';

const connection = { address: 'https://my-mac.tail1a2b3c.ts.net', password: 'correct horse battery' };

function respond(status: number, body: unknown) {
  return Promise.resolve(new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } }));
}

describe('testConnection', () => {
  afterEach(() => jest.restoreAllMocks());

  it('tells an unreachable Mac apart from a wrong password', async () => {
    jest.spyOn(globalThis, 'fetch').mockRejectedValue(new TypeError('Network request failed'));
    const result = await testConnection(connection);
    expect(result).toMatchObject({ ok: false, field: 'address' });
    expect(!result.ok && result.message).toContain("Can't reach my-mac.tail1a2b3c.ts.net");
  });

  it('notices an address that is some other server', async () => {
    jest.spyOn(globalThis, 'fetch').mockImplementation(() => respond(200, { status: 'ok' }));
    expect(await testConnection(connection)).toMatchObject({ ok: false, field: 'address' });
  });

  it('reports a wrong password', async () => {
    jest.spyOn(globalThis, 'fetch').mockImplementation((url) =>
      String(url).endsWith('/health') ? respond(200, { status: 'ok', service: 'comfyui-prompt-server' }) : respond(401, { error: 'Wrong password.' }),
    );
    expect(await testConnection(connection)).toEqual({ ok: false, field: 'password', message: 'Wrong password.' });
  });

  it('sends the password on every call, and nothing before it is checked', async () => {
    const fetch = jest.spyOn(globalThis, 'fetch').mockImplementation((url) =>
      String(url).endsWith('/health')
        ? respond(200, { status: 'ok', service: 'comfyui-prompt-server' })
        : respond(200, { title: 'Qwen', comfy: true, progress: true, seed: 'random', maxPromptChars: 20000, version: '2.0.0' }),
    );
    const result = await testConnection(connection);
    expect(result).toMatchObject({ ok: true, info: { title: 'Qwen' } });
    const [healthUrl, healthInit] = fetch.mock.calls[0];
    expect(healthUrl).toBe('https://my-mac.tail1a2b3c.ts.net/health');
    expect(JSON.stringify(healthInit)).not.toContain(connection.password);
    expect((fetch.mock.calls[1][1]?.headers as Record<string, string>).Authorization).toBe('Bearer correct horse battery');
  });
});

describe('api', () => {
  afterEach(() => jest.restoreAllMocks());

  it('passes the server error message on', async () => {
    jest.spyOn(globalThis, 'fetch').mockImplementation(() => respond(422, { error: 'ComfyUI returned 400: UNETLoader 238:226: Value not in list' }));
    await expect(api.generate(connection, { prompt: 'a fox', kind: 'generate' })).rejects.toMatchObject({
      status: 422,
      message: expect.stringContaining('UNETLoader'),
    });
  });

  it('sends an edit with its picture and switches', async () => {
    const fetch = jest.spyOn(globalThis, 'fetch').mockImplementation(() => respond(202, { id: 'j2', state: 'queued' }));
    await api.generate(connection, {
      prompt: 'make it night',
      kind: 'edit',
      source: { job: 'j1', index: 0 },
      options: { Realism: false },
    });
    const [url, init] = fetch.mock.calls[0];
    expect(url).toBe(`${connection.address}/api/generate`);
    expect(JSON.parse(String(init?.body))).toEqual({
      prompt: 'make it night',
      kind: 'edit',
      source: { job: 'j1', index: 0 },
      options: { Realism: false },
    });
  });

  it('passes on how many jobs a switch would stop', async () => {
    jest.spyOn(globalThis, 'fetch').mockImplementation(() => respond(409, { error: '2 image jobs are still running.', activeJobs: 2 }));
    await expect(api.switchTo(connection, 'edit')).rejects.toMatchObject({ status: 409, activeJobs: 2 });
  });
});
