import { Alert, Platform } from 'react-native';

/** Ask before something that can't be undone. Resolves true when confirmed. */
export function confirmDestructive(title: string, message: string, action: string): Promise<boolean> {
  if (Platform.OS === 'web') return Promise.resolve(globalThis.confirm(`${title}\n\n${message}`));
  return new Promise((resolve) =>
    Alert.alert(
      title,
      message,
      [
        { text: 'Cancel', style: 'cancel', onPress: () => resolve(false) },
        { text: action, style: 'destructive', onPress: () => resolve(true) },
      ],
      { cancelable: true, onDismiss: () => resolve(false) },
    ),
  );
}

export function showMessage(title: string, message: string): void {
  if (Platform.OS === 'web') globalThis.alert(`${title}\n\n${message}`);
  else Alert.alert(title, message);
}
