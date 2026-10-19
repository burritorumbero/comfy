import * as SecureStore from 'expo-secure-store';
import { Platform } from 'react-native';

// The address and password live in the iOS Keychain, readable after the first unlock and
// never synced to other devices. The web build (used only for testing) has no Keychain.
const options: SecureStore.SecureStoreOptions = { keychainAccessible: SecureStore.AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY };

export async function readSecret(key: string): Promise<string | null> {
  if (Platform.OS === 'web') return globalThis.localStorage?.getItem(key) ?? null;
  return SecureStore.getItemAsync(key, options);
}

export async function writeSecret(key: string, value: string): Promise<void> {
  if (Platform.OS === 'web') return globalThis.localStorage?.setItem(key, value);
  return SecureStore.setItemAsync(key, value, options);
}

export async function deleteSecret(key: string): Promise<void> {
  if (Platform.OS === 'web') return globalThis.localStorage?.removeItem(key);
  return SecureStore.deleteItemAsync(key, options);
}

export async function readJson<T>(key: string): Promise<T | null> {
  try {
    const raw = await readSecret(key);
    return raw ? (JSON.parse(raw) as T) : null;
  } catch {
    return null;
  }
}
