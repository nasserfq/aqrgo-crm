// Phone notifications: ask permission once signed in, send the Expo push token
// to the server, and open the right row when a notification is tapped.
import Constants from 'expo-constants';
import * as Device from 'expo-device';
import * as Notifications from 'expo-notifications';
import { router } from 'expo-router';
import { useEffect } from 'react';
import { Platform } from 'react-native';

import type { Api } from './api';
import * as storage from './storage';

const PUSH_KEY = 'aqrgo.push';

/** Registers this phone for new-lead notifications. Quietly does nothing when
 * the person says no, on a simulator, or before the app is linked to EAS. */
export async function registerForPush(api: Api): Promise<void> {
  if (Platform.OS !== 'ios' && Platform.OS !== 'android') return;
  if (!Device.isDevice) return;
  const projectId = Constants.expoConfig?.extra?.eas?.projectId as string | undefined;
  if (!projectId) return;

  if (Platform.OS === 'android') {
    await Notifications.setNotificationChannelAsync('leads', {
      name: 'العروض الجديدة',
      importance: Notifications.AndroidImportance.HIGH,
    });
  }
  let { status } = await Notifications.getPermissionsAsync();
  if (status !== 'granted') {
    ({ status } = await Notifications.requestPermissionsAsync());
  }
  if (status !== 'granted') return;

  const { data: token } = await Notifications.getExpoPushTokenAsync({ projectId });
  await api.registerDevice(token, Platform.OS);
  await storage.setItem(PUSH_KEY, token);
}

/** On sign-out: this phone stops getting that account's notifications. */
export async function unregisterFromPush(api: Api): Promise<void> {
  const token = await storage.getItem(PUSH_KEY).catch(() => null);
  if (!token) return;
  await api.unregisterDevice(token).catch(() => {});
  await storage.deleteItem(PUSH_KEY).catch(() => {});
}

/** Tapping a "new lead" notification opens that row. */
export function useOpenTappedNotification(signedIn: boolean) {
  const response = Notifications.useLastNotificationResponse();
  useEffect(() => {
    if (!signedIn || !response) return;
    const id = response.notification?.request?.content?.data?.listing_id;
    if (typeof id === 'number') {
      router.push({ pathname: '/listings/[id]', params: { id: String(id) } });
    }
  }, [response, signedIn]);
}
