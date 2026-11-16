// The whole app in Jest, through Expo Router, against a fake server.
import { fireEvent, renderRouter, screen, waitFor } from 'expo-router/testing-library';

jest.mock('expo-secure-store', () => {
  const store = new Map<string, string>();
  return {
    __store: store,
    getItemAsync: jest.fn(async (k: string) => store.get(k) ?? null),
    setItemAsync: jest.fn(async (k: string, v: string) => void store.set(k, v)),
    deleteItemAsync: jest.fn(async (k: string) => void store.delete(k)),
  };
});

jest.mock('expo-font', () => ({ ...jest.requireActual('expo-font'), useFonts: () => [true, null] }));

let mockTapped: unknown = null;
jest.mock('expo-notifications', () => ({
  useLastNotificationResponse: () => mockTapped,
  AndroidImportance: { HIGH: 6 },
}));

const secure = jest.requireMock('expo-secure-store').__store as Map<string, string>;

type Req = { method: string; path: string; body?: any; auth?: string };

function row(id: number, extra: Record<string, string> = {}) {
  return {
    id,
    ref_no: id,
    number: String(id),
    offer_type: 'أرض للبيع',
    plan_no: '2456',
    plot_no: '',
    area: '625',
    frontages: '',
    location: 'حي الملقا',
    price: '',
    buyer: '',
    seller: '',
    broker: '',
    rooms: '',
    bathrooms: '',
    floors: '',
    units: '',
    income: '',
    details: '',
    listed_on: '2026-11-13',
    status: 'available',
    status_label: 'متاح',
    date_hijri: 'الجمعة 22 جمادى الأولى 1448هـ',
    date_gregorian: '13-11-2026',
    source: 'web',
    ...extra,
  };
}

let requests: Req[] = [];
let rows: ReturnType<typeof row>[] = [];

function reply(status: number, body?: unknown) {
  return Promise.resolve({
    ok: status < 300,
    status,
    text: async () => (body === undefined ? '' : JSON.stringify(body)),
  } as Response);
}

beforeEach(() => {
  mockTapped = null;
  secure.clear();
  requests = [];
  rows = [row(1)];
  global.fetch = jest.fn((url: string, init: RequestInit) => {
    const u = new URL(url);
    const req: Req = {
      method: init.method ?? 'GET',
      path: u.pathname + u.search,
      body: init.body ? JSON.parse(init.body as string) : undefined,
      auth: (init.headers as Record<string, string>)?.Authorization,
    };
    requests.push(req);
    const signedIn = req.auth === 'Bearer tok';
    if (req.path === '/api/v1/auth/code') return reply(204);
    if (req.path === '/api/v1/auth/verify') {
      return req.body.code === '123456'
        ? reply(200, { token: 'tok', user: { id: 1, email: req.body.email, name: 'nasser' } })
        : reply(401, { error: 'الرمز غير صحيح أو منتهي' });
    }
    if (req.path === '/api/v1/auth/options') return reply(200, { google: true });
    if (!signedIn) return reply(401, { error: 'سجّل الدخول من جديد' });
    const one = u.pathname.match(/^\/api\/v1\/listings\/(\d+)$/);
    if (one && req.method === 'GET') {
      const r = rows.find((x) => x.id === Number(one[1]));
      return r ? reply(200, r) : reply(404, { error: 'السجل غير موجود' });
    }
    if (req.path.startsWith('/api/v1/listings') && req.method === 'GET') {
      const status = u.searchParams.get('status');
      return reply(200, { listings: rows.filter((r) => !status || r.status === status) });
    }
    if (req.path === '/api/v1/listings' && req.method === 'POST') {
      if (!req.body.offer_type && !req.body.location && !req.body.plan_no) {
        return reply(422, { error: 'اكتب نوع العرض أو الموقع أو رقم المخطط على الأقل' });
      }
      const r = row(rows.length + 1, req.body);
      rows = [r, ...rows];
      return reply(201, r);
    }
    if (req.path === '/api/v1/auth/logout') return reply(204);
    if (req.path === '/api/v1/listings/extract') {
      return reply(200, {
        listings: req.body.text.includes('للبيع')
          ? [{ offer_type: 'أرض للبيع', location: 'حي الياسمين', plan_no: '3011', listed_on: null }]
          : [],
      });
    }
    if (req.path === '/api/v1/me' && req.method === 'DELETE') {
      rows = [];
      return reply(204);
    }
    return reply(404, { error: 'السجل غير موجود' });
  }) as unknown as typeof fetch;
});

it('signs in with an emailed code, keeps the token safe, and shows the table', async () => {
  await renderRouter('./src/app', { initialUrl: '/' });

  // Signed out: the sign-in screen, whatever the starting URL.
  await fireEvent.changeText(await screen.findByTestId('email'), ' nasser@example.com ');
  await fireEvent.press(screen.getByTestId('send-code'));
  expect(await screen.findByText(/أرسلنا رمزاً/)).toBeTruthy();
  expect(requests.find((r) => r.path === '/api/v1/auth/code')?.body).toEqual({ email: 'nasser@example.com' });

  await fireEvent.changeText(screen.getByTestId('code'), '000000');
  await fireEvent.press(screen.getByTestId('verify'));
  expect(await screen.findByText('الرمز غير صحيح أو منتهي')).toBeTruthy();

  await fireEvent.changeText(screen.getByTestId('code'), '123456');
  await fireEvent.press(screen.getByTestId('verify'));

  expect(await screen.findByTestId('listing-1')).toBeTruthy();
  expect(screen.getByText('حي الملقا')).toBeTruthy();
  expect(screen.getByText('العروض: 1')).toBeTruthy();
  expect(secure.get('aqrgo.token')).toBe('tok');
});

it('opens straight on the table when a token is saved, and filters by status', async () => {
  secure.set('aqrgo.token', 'tok');
  secure.set('aqrgo.user', JSON.stringify({ id: 1, email: 'n@x.com', name: 'n' }));
  rows = [row(2, { status: 'sold', status_label: 'مباع', location: 'حي النرجس' }), row(1)];

  await renderRouter('./src/app', { initialUrl: '/' });
  expect(await screen.findByTestId('listing-2')).toBeTruthy();

  await fireEvent.press(screen.getByTestId('filter-sold'));
  await waitFor(() => expect(screen.queryByTestId('listing-1')).toBeNull());
  expect(await screen.findByText('حي النرجس')).toBeTruthy();
  expect(requests.at(-1)?.path).toBe('/api/v1/listings?status=sold');
});

it('adds a row and shows the server message when it is incomplete', async () => {
  secure.set('aqrgo.token', 'tok');
  secure.set('aqrgo.user', JSON.stringify({ id: 1, email: 'n@x.com', name: 'n' }));

  await renderRouter('./src/app', { initialUrl: '/' });
  await fireEvent.press(await screen.findByTestId('add'));

  await fireEvent.press(await screen.findByTestId('save'));
  expect(await screen.findByText(/اكتب نوع العرض/)).toBeTruthy();

  await fireEvent.press(screen.getByText('فيلا للبيع'));
  await fireEvent.changeText(screen.getByTestId('field-location'), 'حي حطين');
  await fireEvent.press(screen.getByTestId('status-reserved'));
  await fireEvent.press(screen.getByTestId('save'));

  expect(await screen.findByText('حي حطين')).toBeTruthy();
  const created = requests.find((r) => r.method === 'POST' && r.path === '/api/v1/listings' && r.body.location);
  expect(created?.body).toMatchObject({ offer_type: 'فيلا للبيع', location: 'حي حطين', status: 'reserved' });
});

it('goes back to sign-in when the server says the token expired', async () => {
  secure.set('aqrgo.token', 'expired');
  secure.set('aqrgo.user', JSON.stringify({ id: 1, email: 'n@x.com', name: 'n' }));

  await renderRouter('./src/app', { initialUrl: '/' });
  expect(await screen.findByTestId('email')).toBeTruthy();
  expect(secure.has('aqrgo.token')).toBe(false);
});

it('opens the new row when its notification is mockTapped', async () => {
  secure.set('aqrgo.token', 'tok');
  secure.set('aqrgo.user', JSON.stringify({ id: 1, email: 'n@x.com', name: 'n' }));
  rows = [row(5, { location: 'حي العارض' })];
  mockTapped = { notification: { request: { content: { data: { listing_id: 5 } } } } };

  await renderRouter('./src/app', { initialUrl: '/' });

  expect(await screen.findByDisplayValue('حي العارض')).toBeTruthy();
  await waitFor(() => expect(requests.some((r) => r.path === '/api/v1/listings/5')).toBe(true));
});

it('pastes a lead, lets the AI fill the form, and saves after review', async () => {
  secure.set('aqrgo.token', 'tok');
  secure.set('aqrgo.user', JSON.stringify({ id: 1, email: 'n@x.com', name: 'n' }));

  await renderRouter('./src/app', { initialUrl: '/' });
  await fireEvent.press(await screen.findByTestId('add'));

  await fireEvent.changeText(await screen.findByTestId('paste'), 'كلام عام');
  await fireEvent.press(screen.getByTestId('read'));
  expect(await screen.findByText(/لم أجد عرضاً/)).toBeTruthy();

  await fireEvent.changeText(screen.getByTestId('paste'), 'للبيع أرض في الياسمين مخطط 3011');
  await fireEvent.press(screen.getByTestId('read'));
  expect(await screen.findByDisplayValue('حي الياسمين')).toBeTruthy();
  expect(screen.getByDisplayValue('3011')).toBeTruthy();
  expect(requests.some((r) => r.method === 'POST' && r.path === '/api/v1/listings')).toBe(false);

  await fireEvent.press(screen.getByTestId('save'));
  expect(await screen.findByText('حي الياسمين')).toBeTruthy();
  const created = requests.find((r) => r.method === 'POST' && r.path === '/api/v1/listings');
  expect(created?.body).toMatchObject({ offer_type: 'أرض للبيع', plan_no: '3011', status: 'available' });
});

it('shows Google sign-in when the server offers it', async () => {
  await renderRouter('./src/app', { initialUrl: '/' });
  expect(await screen.findByTestId('google')).toBeTruthy();
});

it('deletes the account after confirming, and goes back to sign-in', async () => {
  secure.set('aqrgo.token', 'tok');
  secure.set('aqrgo.user', JSON.stringify({ id: 1, email: 'n@x.com', name: 'n' }));
  const alert = jest.spyOn(require('react-native').Alert, 'alert').mockImplementation(
    (...args: unknown[]) => {
      const buttons = args[2] as { style?: string; onPress?: () => void }[];
      buttons.find((b) => b.style === 'destructive')?.onPress?.();
    },
  );

  await renderRouter('./src/app', { initialUrl: '/' });
  await fireEvent.press(await screen.findByTestId('account'));
  expect(await screen.findByText('n@x.com')).toBeTruthy();
  await fireEvent.press(screen.getByTestId('delete-account'));

  expect(await screen.findByTestId('email')).toBeTruthy();
  expect(requests.some((r) => r.method === 'DELETE' && r.path === '/api/v1/me')).toBe(true);
  expect(secure.has('aqrgo.token')).toBe(false);
  alert.mockRestore();
});
