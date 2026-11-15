// Holds the sign-in token in the phone's secure storage (Keychain / Keystore)
// and gives every screen an API client that sends it.
import Constants from 'expo-constants';
import * as storage from './storage';
import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';

import { createApi, type Api, type User } from './api';
import { unregisterFromPush } from './notifications';

const TOKEN_KEY = 'aqrgo.token';
const USER_KEY = 'aqrgo.user';

export const API_URL: string =
  (Constants.expoConfig?.extra?.apiUrl as string | undefined) ?? 'https://aqrgo.com';

type Session = {
  ready: boolean;
  user: User | null;
  api: Api;
  signIn: (token: string, user: User) => Promise<void>;
  signOut: () => Promise<void>;
};

const SessionContext = createContext<Session | null>(null);

export function SessionProvider({ children, api: injected }: { children: ReactNode; api?: Api }) {
  const token = useRef<string | null>(null);
  const [user, setUser] = useState<User | null>(null);
  const [ready, setReady] = useState(false);

  const clear = async () => {
    token.current = null;
    setUser(null);
    await Promise.all([
      storage.deleteItem(TOKEN_KEY),
      storage.deleteItem(USER_KEY),
    ]).catch(() => {});
  };

  const api = useMemo(
    () =>
      injected ??
      createApi({
        baseUrl: API_URL,
        getToken: () => token.current,
        onUnauthorized: () => void clear(),
      }),
    [injected],
  );

  useEffect(() => {
    (async () => {
      try {
        const [t, u] = await Promise.all([
          storage.getItem(TOKEN_KEY),
          storage.getItem(USER_KEY),
        ]);
        if (t && u) {
          token.current = t;
          setUser(JSON.parse(u));
        }
      } catch {
        // Unreadable storage: start signed out.
      } finally {
        setReady(true);
      }
    })();
  }, []);

  const value: Session = {
    ready,
    user,
    api,
    signIn: async (t, u) => {
      token.current = t;
      await storage.setItem(TOKEN_KEY, t);
      await storage.setItem(USER_KEY, JSON.stringify(u));
      setUser(u);
    },
    signOut: async () => {
      await unregisterFromPush(api);
      await api.logout().catch(() => {});
      await clear();
    },
  };

  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}

export function useSession(): Session {
  const s = useContext(SessionContext);
  if (!s) throw new Error('useSession outside SessionProvider');
  return s;
}
