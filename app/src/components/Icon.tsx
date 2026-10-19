import { SymbolView, type SFSymbol, type SymbolWeight } from 'expo-symbols';
import { Text } from 'react-native';

import { colors } from '../lib/theme';

type Props = { name: SFSymbol; size?: number; color?: string; weight?: SymbolWeight; fallback: string };

/** An SF Symbol on iOS; the `fallback` glyph elsewhere (the web build used for testing). */
export function Icon({ name, size = 20, color = colors.text, weight = 'regular', fallback }: Props) {
  return (
    <SymbolView
      name={name}
      size={size}
      tintColor={color}
      weight={weight}
      fallback={<Text style={{ color, fontSize: size * 0.85, lineHeight: size }}>{fallback}</Text>}
    />
  );
}
