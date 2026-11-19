import { Alexandria_800ExtraBold } from '@expo-google-fonts/alexandria';
import {
  IBMPlexSansArabic_400Regular,
  IBMPlexSansArabic_500Medium,
  IBMPlexSansArabic_700Bold,
} from '@expo-google-fonts/ibm-plex-sans-arabic';
import { focusManager, QueryClient, QueryClientProvider, useQueryClient } from '@tanstack/react-query';
import { useFonts } from 'expo-font';
import { SplashScreen, Stack } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { useEffect, useState } from 'react';
import { AppState, I18nManager, Platform } from 'react-native';

import { registerForPush, useOpenTappedNotification } from '../lib/notifications';
import { SessionProvider, useSession } from '../lib/session';
import { fonts, useColors } from '../lib/theme';

// Arabic first: the whole app is laid out right to left (also forced natively
// by the expo-localization plugin in app.json).
I18nManager.allowRTL(true);

// React Query only knows about browser focus. On a phone, coming back to the
// app (say from WhatsApp after sending a listing) counts as focus, so stale
// lists refetch without a pull to refresh.
if (Platform.OS !== 'web') {
  focusManager.setEventListener((setFocused) => {
    const sub = AppState.addEventListener('change', (s) => setFocused(s === 'active'));
    return () => sub.remove();
  });
}
I18nManager.forceRTL(true);
if (Platform.OS === 'web' && typeof document !== 'undefined') document.documentElement.dir = 'rtl';

SplashScreen.preventAutoHideAsync().catch(() => {});

function Screens() {
  const { ready, user, api } = useSession();
  const c = useColors();
  const queryClient = useQueryClient();
  // Nothing from one account stays on screen after signing out.
  useEffect(() => {
    if (!user) queryClient.clear();
  }, [user, queryClient]);
  // Signed in: this phone gets a notification for each new lead from Telegram.
  useEffect(() => {
    if (user) registerForPush(api).catch(() => {});
  }, [user, api]);
  useOpenTappedNotification(!!user && ready);
  const [fontsLoaded] = useFonts({
    IBMPlexSansArabic_400Regular,
    IBMPlexSansArabic_500Medium,
    IBMPlexSansArabic_700Bold,
    Alexandria_800ExtraBold,
  });

  const loaded = ready && fontsLoaded;
  useEffect(() => {
    if (loaded) SplashScreen.hideAsync().catch(() => {});
  }, [loaded]);
  if (!loaded) return null;

  return (
    <>
      <StatusBar style="auto" />
      <Stack
        screenOptions={{
          headerStyle: { backgroundColor: c.surface },
          headerTintColor: c.accent,
          headerTitleStyle: { fontFamily: fonts.bold, color: c.ink },
          headerShadowVisible: false,
          contentStyle: { backgroundColor: c.bg },
          headerBackButtonDisplayMode: 'minimal',
        }}>
        <Stack.Protected guard={!user}>
          <Stack.Screen name="login" options={{ headerShown: false }} />
        </Stack.Protected>
        <Stack.Protected guard={!!user}>
          <Stack.Screen name="index" options={{ title: 'العروض' }} />
          <Stack.Screen name="listings/new" options={{ title: 'إضافة عرض', presentation: 'modal' }} />
          <Stack.Screen name="listings/[id]" options={{ title: 'تعديل العرض' }} />
          <Stack.Screen name="account" options={{ title: 'حسابي' }} />
        </Stack.Protected>
      </Stack>
    </>
  );
}

export default function RootLayout() {
  const [queryClient] = useState(
    () => new QueryClient({ defaultOptions: { queries: { retry: 1, staleTime: 15_000 } } }),
  );
  return (
    <QueryClientProvider client={queryClient}>
      <SessionProvider>
        <Screens />
      </SessionProvider>
    </QueryClientProvider>
  );
}
