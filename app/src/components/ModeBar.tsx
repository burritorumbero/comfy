import * as Haptics from 'expo-haptics';
import type { SFSymbol } from 'expo-symbols';
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from 'react-native';

import type { Mode, Services } from '../lib/api';
import { modeLabel, selectedMode } from '../lib/services';
import { useRequestSwitch } from '../lib/servicesStore';
import { colors, radius } from '../lib/theme';
import { Icon } from './Icon';

const icons: Record<Mode, { name: SFSymbol; fallback: string }> = {
  generate: { name: 'sparkles', fallback: '✦' },
  edit: { name: 'wand.and.stars', fallback: '✎' },
  llm: { name: 'text.bubble', fallback: '…' },
};

const hints: Record<Mode, string> = {
  generate: 'Loads the image model on your Mac.',
  edit: 'Loads the edit model on your Mac.',
  llm: 'Frees ComfyUI and loads the LLM, for Hermes.',
};

/**
 * What the Mac's memory is for: one segment per mode. Choosing one frees the current model before
 * the next loads, so two heavy models are never in memory together.
 */
export function ModeBar({ services, onSwitched }: { services: Services; onSwitched?: (to: Mode) => void }) {
  const requestSwitch = useRequestSwitch();
  const selected = selectedMode(services);
  const switching = services.switching != null;

  const choose = async (mode: Mode) => {
    if (switching || mode === selected) return;
    void Haptics.selectionAsync();
    if (await requestSwitch(mode)) onSwitched?.(mode);
  };

  return (
    <View style={styles.bar} accessibilityRole="tablist">
      {services.modes.map((mode) => {
        const on = mode === selected;
        const tint = on ? colors.accentText : colors.muted;
        return (
          <Pressable
            key={mode}
            onPress={() => void choose(mode)}
            disabled={switching && !on}
            accessibilityRole="tab"
            accessibilityLabel={modeLabel[mode]}
            accessibilityHint={on ? undefined : hints[mode]}
            accessibilityState={{ selected: on, busy: on && switching, disabled: switching && !on }}
            style={({ pressed }) => [styles.segment, on && styles.on, pressed && !on && styles.pressed, switching && !on && styles.off]}
          >
            {on && switching ? (
              <ActivityIndicator size="small" color={tint} />
            ) : (
              <Icon name={icons[mode].name} size={14} color={tint} fallback={icons[mode].fallback} />
            )}
            <Text style={[styles.label, { color: tint }]} maxFontSizeMultiplier={1.3}>
              {modeLabel[mode]}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  bar: {
    flexDirection: 'row',
    gap: 4,
    marginHorizontal: 12,
    marginTop: 10,
    marginBottom: 8,
    padding: 3,
    borderRadius: radius.control,
    backgroundColor: colors.surface,
  },
  segment: {
    flex: 1,
    minHeight: 34,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 6,
    borderRadius: radius.control - 3,
  },
  on: { backgroundColor: colors.accent },
  pressed: { backgroundColor: colors.card },
  off: { opacity: 0.45 },
  label: { fontSize: 14, fontWeight: '600' },
});
