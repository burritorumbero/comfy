import { Pressable, StyleSheet, Text, View } from 'react-native';

import { colors, maxFontScale, radius } from '../lib/theme';

type Props = { tone: 'warn' | 'danger'; text: string; action?: string; onAction?: () => void };

/** A calm, persistent note about the connection, with at most one way to fix it. */
export function Banner({ tone, text, action, onAction }: Props) {
  const tint = tone === 'danger' ? colors.danger : colors.accent;
  return (
    <View style={[styles.banner, { borderColor: tint }]} accessibilityRole="alert">
      <Text style={styles.text} maxFontSizeMultiplier={maxFontScale}>
        {text}
      </Text>
      {action && onAction ? (
        <Pressable onPress={onAction} accessibilityRole="button" hitSlop={8}>
          <Text style={[styles.action, { color: tint }]} maxFontSizeMultiplier={maxFontScale}>
            {action}
          </Text>
        </Pressable>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  banner: {
    marginHorizontal: 12,
    marginBottom: 8,
    padding: 12,
    gap: 8,
    borderRadius: radius.control,
    borderWidth: StyleSheet.hairlineWidth,
    backgroundColor: colors.surface,
  },
  text: { color: colors.text, fontSize: 14, lineHeight: 19 },
  action: { fontSize: 14, fontWeight: '600' },
});
