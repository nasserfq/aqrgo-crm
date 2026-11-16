// The web build (used only for quick previews) has no push notifications.
import type { Api } from './api';

export async function registerForPush(_api: Api): Promise<void> {}
export async function unregisterFromPush(_api: Api): Promise<void> {}
export function useOpenTappedNotification(_signedIn: boolean) {}
