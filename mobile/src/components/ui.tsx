// Small building blocks in the lavender style: buttons, inputs, labels, notes.
import type { ReactNode } from 'react';
import {
  ActivityIndicator,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
  type TextInputProps,
  type TextProps,
} from 'react-native';

import { fonts, radius, useColors } from '../lib/theme';

export function T({ style, ...p }: TextProps) {
  const c = useColors();
  return <Text {...p} style={[{ color: c.ink, fontFamily: fonts.body, fontSize: 16, writingDirection: 'rtl' }, style]} />;
}

export function Title({ children }: { children: ReactNode }) {
  return (
    <T style={{ fontFamily: fonts.display, fontSize: 28, lineHeight: 42 }} accessibilityRole="header">
      {children}
    </T>
  );
}

type ButtonProps = {
  label: string;
  onPress: () => void;
  kind?: 'primary' | 'outline' | 'danger';
  busy?: boolean;
  disabled?: boolean;
  testID?: string;
};

export function Button({ label, onPress, kind = 'primary', busy, disabled, testID }: ButtonProps) {
  const c = useColors();
  const bg = kind === 'primary' ? c.primary : kind === 'danger' ? c.dangerSoft : c.surface;
  const fg = kind === 'primary' ? c.primaryInk : kind === 'danger' ? c.danger : c.ink;
  const border = kind === 'outline' ? c.lineStrong : kind === 'danger' ? c.danger : bg;
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityState={{ disabled: !!(disabled || busy), busy: !!busy }}
      disabled={disabled || busy}
      onPress={onPress}
      style={({ pressed }) => [
        styles.button,
        { backgroundColor: bg, borderColor: border, opacity: disabled ? 0.5 : 1 },
        pressed && { transform: [{ scale: 0.98 }] },
      ]}>
      {busy ? (
        <ActivityIndicator color={fg} />
      ) : (
        <Text style={{ color: fg, fontFamily: fonts.bold, fontSize: 17 }}>{label}</Text>
      )}
    </Pressable>
  );
}

export function Field({ label, ...p }: TextInputProps & { label?: string }) {
  const c = useColors();
  return (
    <View style={{ gap: 6 }}>
      {label ? <T style={{ fontFamily: fonts.medium, fontSize: 15 }}>{label}</T> : null}
      <TextInput
        placeholderTextColor={c.muted}
        accessibilityLabel={label ?? p.placeholder}
        {...p}
        style={[
          styles.input,
          { backgroundColor: c.surface, borderColor: c.lineStrong, color: c.ink, fontFamily: fonts.body },
          p.multiline && { minHeight: 96, textAlignVertical: 'top' },
          p.style,
        ]}
      />
    </View>
  );
}

export function Note({ kind = 'error', children }: { kind?: 'error' | 'info'; children: ReactNode }) {
  const c = useColors();
  const bg = kind === 'error' ? c.dangerSoft : c.accentSoft;
  const fg = kind === 'error' ? c.danger : c.accentDeep;
  const edge = kind === 'error' ? c.danger : c.accent;
  return (
    <View
      accessibilityRole="alert"
      style={{ backgroundColor: bg, borderStartWidth: 3, borderStartColor: edge, padding: 12, borderRadius: radius }}>
      <T style={{ color: fg }}>{children}</T>
    </View>
  );
}

const styles = StyleSheet.create({
  button: {
    minHeight: 52,
    borderRadius: radius,
    borderWidth: 1,
    alignItems: 'center',
    justifyContent: 'center',
    paddingHorizontal: 16,
  },
  input: {
    writingDirection: 'rtl',
    minHeight: 52,
    borderWidth: 1,
    borderRadius: radius,
    paddingHorizontal: 14,
    paddingVertical: 10,
    fontSize: 17,
  },
});
