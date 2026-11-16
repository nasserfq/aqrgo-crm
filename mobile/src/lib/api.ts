// Client for the AQRGO JSON API (docs/api.md in the repo root).
// Pure TypeScript with an injectable fetch, so it is unit-tested without a phone.

export type Status = 'available' | 'reserved' | 'sold';

export type User = { id: number; email: string; name: string };

export type Listing = {
  id: number;
  ref_no: number;
  number: string;
  offer_type: string;
  plan_no: string;
  plot_no: string;
  area: string;
  frontages: string;
  location: string;
  price: string;
  buyer: string;
  seller: string;
  broker: string;
  rooms: string;
  bathrooms: string;
  floors: string;
  units: string;
  income: string;
  details: string;
  listed_on: string;
  status: Status;
  status_label: string;
  date_hijri: string;
  date_gregorian: string;
  source: string;
};

/** What the app sends when creating or editing a row. Missing fields are empty. */
export type ListingInput = Partial<
  Omit<Listing, 'id' | 'ref_no' | 'number' | 'status_label' | 'date_hijri' | 'date_gregorian' | 'source'>
>;

export type Meta = {
  fields: { key: keyof ListingInput; label: string }[];
  offer_types: string[];
  statuses: { value: Status; label: string }[];
};

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

const OFFLINE = 'تعذر الاتصال بالخادم، تحقق من الإنترنت';
const UNKNOWN = 'حدث خطأ، حاول مرة أخرى';

type Options = {
  baseUrl: string;
  getToken: () => string | null;
  fetch?: (url: string, init: RequestInit) => Promise<Response>;
  /** Called on a 401 for a signed-in call, so the app can go back to sign-in. */
  onUnauthorized?: () => void;
};

export function createApi(opts: Options) {
  const base = opts.baseUrl.replace(/\/+$/, '');
  const doFetch = opts.fetch ?? ((url, init) => fetch(url, init));

  async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = { Accept: 'application/json' };
    const token = opts.getToken();
    if (token) headers.Authorization = `Bearer ${token}`;
    if (body !== undefined) headers['Content-Type'] = 'application/json';

    let res: Response;
    try {
      res = await doFetch(base + path, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
      });
    } catch {
      throw new ApiError(0, OFFLINE);
    }

    const text = await res.text();
    let data: unknown = undefined;
    if (text) {
      try {
        data = JSON.parse(text);
      } catch {
        data = undefined;
      }
    }
    if (!res.ok) {
      if (res.status === 401 && token) opts.onUnauthorized?.();
      const message =
        data && typeof data === 'object' && 'error' in data && typeof data.error === 'string'
          ? data.error
          : UNKNOWN;
      throw new ApiError(res.status, message);
    }
    return data as T;
  }

  return {
    requestCode: (email: string) => request<void>('POST', '/api/v1/auth/code', { email }),
    verifyCode: (email: string, code: string) =>
      request<{ token: string; user: User }>('POST', '/api/v1/auth/verify', { email, code }),
    logout: () => request<void>('POST', '/api/v1/auth/logout'),
    me: () => request<User>('GET', '/api/v1/me'),

    listListings: async (filter: { q?: string; status?: Status | '' }) => {
      const params: string[] = [];
      const q = filter.q?.trim();
      if (q) params.push(`q=${encodeURIComponent(q)}`);
      if (filter.status) params.push(`status=${filter.status}`);
      const query = params.length ? `?${params.join('&')}` : '';
      const r = await request<{ listings: Listing[] }>('GET', `/api/v1/listings${query}`);
      return r.listings;
    },
    meta: () => request<Meta>('GET', '/api/v1/listings/meta'),
    getListing: (id: number) => request<Listing>('GET', `/api/v1/listings/${id}`),
    createListing: (input: ListingInput) => request<Listing>('POST', '/api/v1/listings', input),
    updateListing: (id: number, input: ListingInput) =>
      request<Listing>('PUT', `/api/v1/listings/${id}`, input),
    deleteListing: (id: number) => request<void>('DELETE', `/api/v1/listings/${id}`),

    registerDevice: (token: string, platform: 'ios' | 'android') =>
      request<void>('POST', '/api/v1/push/devices', { token, platform }),
    unregisterDevice: (token: string) => request<void>('DELETE', '/api/v1/push/devices', { token }),
  };
}

export type Api = ReturnType<typeof createApi>;
