// Secrets live in the Keychain (iOS) / Keystore (Android).
import * as SecureStore from 'expo-secure-store';

export const getItem = (k: string) => SecureStore.getItemAsync(k);
export const setItem = (k: string, v: string) => SecureStore.setItemAsync(k, v);
export const deleteItem = (k: string) => SecureStore.deleteItemAsync(k);
