import { useEffect, useState } from 'react';
import { AppState, StyleSheet, Text, View } from 'react-native';

import { usePreferences } from '../lib/preferences';
import { colors } from '../lib/theme';

/**
 * Covers the app whenever it isn't in the foreground, so the app switcher and Control Center
 * show a blank card instead of your images. On by default; Settings can turn it off.
 */
export function PrivacyCover() {
  const { hideInSwitcher } = usePreferences();
  const [state, setState] = useState(AppState.currentState);
  useEffect(() => {
    const subscription = AppState.addEventListener('change', setState);
    return () => subscription.remove();
  }, []);
  if (!hideInSwitcher || state === 'active') return null;
  return (
    <View style={styles.cover} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      <Text style={styles.name}>Prompt</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  cover: { ...StyleSheet.absoluteFill, backgroundColor: colors.background, alignItems: 'center', justifyContent: 'center' },
  name: { color: colors.faint, fontSize: 17, fontWeight: '600' },
});
