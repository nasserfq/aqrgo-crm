import { ApiError, createApi } from '../src/lib/api';

type Call = { url: string; init: RequestInit };

function fakeFetch(responses: Array<{ status: number; body?: unknown }>) {
  const calls: Call[] = [];
  const fn = jest.fn(async (url: string, init: RequestInit) => {
    calls.push({ url, init });
    const r = responses.shift();
    if (!r) throw new Error('unexpected call');
    return {
      ok: r.status >= 200 && r.status < 300,
      status: r.status,
      text: async () => (r.body === undefined ? '' : JSON.stringify(r.body)),
    } as Response;
  });
  return { fn, calls };
}

const row = {
  id: 7,
  ref_no: 1,
  number: '١',
  offer_type: 'أرض للبيع',
  location: 'حي الملقا',
  status: 'available',
  status_label: 'متاح',
};

describe('api client', () => {
  it('requests a code and verifies it, returning the token', async () => {
    const { fn, calls } = fakeFetch([
      { status: 204 },
      { status: 200, body: { token: 't0k', user: { id: 1, email: 'n@x.com', name: 'n' } } },
    ]);
    const api = createApi({ baseUrl: 'https://aqrgo.com', getToken: () => null, fetch: fn });

    await api.requestCode('n@x.com');
    const session = await api.verifyCode('n@x.com', '١٢٣٤٥٦');

    expect(calls[0].url).toBe('https://aqrgo.com/api/v1/auth/code');
    expect(calls[0].init.method).toBe('POST');
    expect(JSON.parse(calls[0].init.body as string)).toEqual({ email: 'n@x.com' });
    expect(JSON.parse(calls[1].init.body as string)).toEqual({ email: 'n@x.com', code: '١٢٣٤٥٦' });
    expect(session.token).toBe('t0k');
    expect(session.user.email).toBe('n@x.com');
  });

  it("confirms the email with Apple's token when Apple hid the email", async () => {
    const { fn, calls } = fakeFetch([
      { status: 200, body: { token: 't0k', user: { id: 1, email: 'n@x.com', name: 'n' } } },
    ]);
    const api = createApi({ baseUrl: 'https://aqrgo.com', getToken: () => null, fetch: fn });

    await api.verifyCode('n@x.com', '123456', 'apple.jwt');

    expect(JSON.parse(calls[0].init.body as string)).toEqual({
      email: 'n@x.com',
      code: '123456',
      apple_identity_token: 'apple.jwt',
    });
  });

  it('sends the bearer token and builds the search query', async () => {
    const { fn, calls } = fakeFetch([{ status: 200, body: { listings: [row] } }]);
    const api = createApi({ baseUrl: 'https://aqrgo.com/', getToken: () => 'abc', fetch: fn });

    const rows = await api.listListings({ q: 'الملقا 12', status: 'sold' });

    expect(rows).toHaveLength(1);
    expect(calls[0].url).toBe(
      'https://aqrgo.com/api/v1/listings?q=%D8%A7%D9%84%D9%85%D9%84%D9%82%D8%A7%2012&status=sold',
    );
    expect((calls[0].init.headers as Record<string, string>).Authorization).toBe('Bearer abc');
  });

  it('leaves out empty filters', async () => {
    const { fn, calls } = fakeFetch([{ status: 200, body: { listings: [] } }]);
    const api = createApi({ baseUrl: 'https://aqrgo.com', getToken: () => 'abc', fetch: fn });
    await api.listListings({ q: '  ', status: '' });
    expect(calls[0].url).toBe('https://aqrgo.com/api/v1/listings');
  });

  it('creates, updates and deletes rows', async () => {
    const { fn, calls } = fakeFetch([
      { status: 201, body: row },
      { status: 200, body: { ...row, status: 'sold' } },
      { status: 204 },
    ]);
    const api = createApi({ baseUrl: 'https://aqrgo.com', getToken: () => 'abc', fetch: fn });

    expect((await api.createListing({ offer_type: 'أرض للبيع' })).id).toBe(7);
    expect((await api.updateListing(7, { status: 'sold' })).status).toBe('sold');
    await api.deleteListing(7);

    expect(calls.map((c) => [c.init.method, c.url])).toEqual([
      ['POST', 'https://aqrgo.com/api/v1/listings'],
      ['PUT', 'https://aqrgo.com/api/v1/listings/7'],
      ['DELETE', 'https://aqrgo.com/api/v1/listings/7'],
    ]);
  });

  it('turns error answers into ApiError with the Arabic message', async () => {
    const { fn } = fakeFetch([{ status: 422, body: { error: 'اكتب نوع العرض' } }]);
    const api = createApi({ baseUrl: 'https://aqrgo.com', getToken: () => 'abc', fetch: fn });

    const err = await api.createListing({}).catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(422);
    expect(err.message).toBe('اكتب نوع العرض');
  });

  it('tells the app when the token is no longer valid', async () => {
    const { fn } = fakeFetch([{ status: 401, body: { error: 'سجّل الدخول من جديد' } }]);
    const onUnauthorized = jest.fn();
    const api = createApi({
      baseUrl: 'https://aqrgo.com',
      getToken: () => 'old',
      fetch: fn,
      onUnauthorized,
    });

    await expect(api.listListings({})).rejects.toThrow('سجّل الدخول من جديد');
    expect(onUnauthorized).toHaveBeenCalled();
  });

  it('gives an Arabic message when the network is down', async () => {
    const fn = jest.fn(async () => {
      throw new TypeError('Network request failed');
    });
    const api = createApi({ baseUrl: 'https://aqrgo.com', getToken: () => 'abc', fetch: fn });
    const err = await api.listListings({}).catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(0);
    expect(err.message).toBe('تعذر الاتصال بالخادم، تحقق من الإنترنت');
  });
});

describe('api client: sign-in options, account and AI', () => {
  it('finishes Google and Apple sign-in and deletes the account', async () => {
    const session = { token: 't', user: { id: 1, email: 'n@x.com', name: 'n' } };
    const { fn, calls } = fakeFetch([
      { status: 200, body: { google: true } },
      { status: 200, body: session },
      { status: 200, body: session },
      { status: 204 },
    ]);
    const api = createApi({ baseUrl: 'https://aqrgo.com', getToken: () => 'abc', fetch: fn });

    expect((await api.authOptions()).google).toBe(true);
    expect((await api.googleSignIn('c0de', 'v3rifier')).token).toBe('t');
    expect((await api.appleSignIn('jwt', 'ناصر')).user.email).toBe('n@x.com');
    await api.deleteAccount();

    expect(calls.map((c) => [c.init.method, c.url])).toEqual([
      ['GET', 'https://aqrgo.com/api/v1/auth/options'],
      ['POST', 'https://aqrgo.com/api/v1/auth/google'],
      ['POST', 'https://aqrgo.com/api/v1/auth/apple'],
      ['DELETE', 'https://aqrgo.com/api/v1/me'],
    ]);
    expect(JSON.parse(calls[1].init.body as string)).toEqual({ code: 'c0de', verifier: 'v3rifier' });
    expect(JSON.parse(calls[2].init.body as string)).toEqual({ identity_token: 'jwt', name: 'ناصر' });
  });

  it('sends pasted text to the AI and returns the drafts', async () => {
    const { fn, calls } = fakeFetch([
      { status: 200, body: { listings: [{ offer_type: 'أرض للبيع', location: 'حي الملقا' }] } },
    ]);
    const api = createApi({ baseUrl: 'https://aqrgo.com', getToken: () => 'abc', fetch: fn });
    const drafts = await api.extractListings('للبيع أرض في الملقا');
    expect(drafts[0].offer_type).toBe('أرض للبيع');
    expect(calls[0].url).toBe('https://aqrgo.com/api/v1/listings/extract');
    expect(JSON.parse(calls[0].init.body as string)).toEqual({ text: 'للبيع أرض في الملقا' });
  });
});
